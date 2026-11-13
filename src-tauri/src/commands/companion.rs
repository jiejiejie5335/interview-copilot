use serde::Serialize;
#[cfg(target_os = "macos")]
use std::ffi::CStr;
use std::{
    net::{IpAddr, Ipv4Addr, UdpSocket},
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Emitter, State};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::broadcast,
    task::JoinHandle,
    time::{self, Duration},
};

const MAX_REQUEST_BYTES: usize = 8 * 1024;
const KEEP_ALIVE_INTERVAL: Duration = Duration::from_secs(15);
type GenerateCallback = Arc<dyn Fn() + Send + Sync>;

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct CompanionSnapshot {
    request_id: String,
    live_transcript: String,
    question: String,
    answer: String,
    answer_language: String,
    status: String,
    done: bool,
    updated_at: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompanionServerInfo {
    running: bool,
    url: String,
    port: u16,
    local_only: bool,
}

#[derive(Default)]
struct CompanionInner {
    task: Option<JoinHandle<()>>,
    port: Option<u16>,
    token: Option<String>,
    sender: Option<broadcast::Sender<String>>,
    snapshot: CompanionSnapshot,
}

#[derive(Clone, Default)]
pub struct CompanionRuntime {
    inner: Arc<Mutex<CompanionInner>>,
}

impl CompanionRuntime {
    async fn start(
        &self,
        token: String,
        on_generate: GenerateCallback,
    ) -> Result<CompanionServerInfo, String> {
        validate_token(&token)?;
        self.stop();

        let listener = TcpListener::bind((Ipv4Addr::UNSPECIFIED, 0))
            .await
            .map_err(|error| format!("无法启动手机同步服务：{error}"))?;
        let port = listener
            .local_addr()
            .map_err(|error| format!("无法读取手机同步端口：{error}"))?
            .port();
        let (sender, _) = broadcast::channel(128);

        {
            let mut inner = self.inner.lock().unwrap_or_else(|error| error.into_inner());
            inner.port = Some(port);
            inner.token = Some(token);
            inner.sender = Some(sender);
            inner.snapshot = CompanionSnapshot {
                status: "等待 AI 回答".to_string(),
                updated_at: unix_timestamp(),
                ..CompanionSnapshot::default()
            };
        }

        let runtime = self.clone();
        let task = tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    break;
                };
                let connection_runtime = runtime.clone();
                let connection_callback = on_generate.clone();
                tokio::spawn(async move {
                    let _ =
                        handle_connection(stream, connection_runtime, connection_callback).await;
                });
            }
        });
        self.inner
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .task = Some(task);

        self.info()
    }

    fn stop(&self) {
        let mut inner = self.inner.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(task) = inner.task.take() {
            task.abort();
        }
        inner.port = None;
        inner.token = None;
        inner.sender = None;
        inner.snapshot = CompanionSnapshot::default();
    }

    fn info(&self) -> Result<CompanionServerInfo, String> {
        let inner = self.inner.lock().unwrap_or_else(|error| error.into_inner());
        let (Some(port), Some(token)) = (inner.port, inner.token.as_deref()) else {
            return Ok(CompanionServerInfo {
                running: false,
                url: String::new(),
                port: 0,
                local_only: false,
            });
        };
        let address = local_ipv4().unwrap_or(Ipv4Addr::LOCALHOST);
        Ok(CompanionServerInfo {
            running: true,
            url: format!("http://{address}:{port}/{token}"),
            port,
            local_only: address.is_loopback(),
        })
    }

    pub fn begin_answer(&self, request_id: &str, question: &str, answer_language: &str) {
        self.update_snapshot(|snapshot| {
            let live_transcript = snapshot.live_transcript.clone();
            *snapshot = CompanionSnapshot {
                request_id: request_id.to_string(),
                live_transcript,
                question: question.to_string(),
                answer: String::new(),
                answer_language: answer_language.to_string(),
                status: "正在生成回答".to_string(),
                done: false,
                updated_at: unix_timestamp(),
            };
        });
    }

    pub fn update_live_transcript(&self, text: &str) {
        self.update_snapshot(|snapshot| {
            snapshot.live_transcript = text.to_string();
            snapshot.updated_at = unix_timestamp();
        });
    }

    pub fn apply_stream(&self, request_id: &str, delta: &str, done: bool, error: Option<&str>) {
        self.update_snapshot(|snapshot| {
            if snapshot.request_id != request_id {
                return;
            }
            if !delta.is_empty() {
                snapshot.answer.push_str(delta);
            }
            snapshot.done = done;
            snapshot.status = match error {
                Some(message) => format!("生成失败：{message}"),
                None if done => "回答已生成".to_string(),
                None => "正在生成回答".to_string(),
            };
            snapshot.updated_at = unix_timestamp();
        });
    }

    fn update_snapshot(&self, update: impl FnOnce(&mut CompanionSnapshot)) {
        let mut inner = self.inner.lock().unwrap_or_else(|error| error.into_inner());
        if inner.sender.is_none() {
            return;
        }
        update(&mut inner.snapshot);
        if let (Some(sender), Ok(payload)) = (
            inner.sender.as_ref(),
            serde_json::to_string(&inner.snapshot),
        ) {
            let _ = sender.send(payload);
        }
    }

    fn token_matches(&self, candidate: &str) -> bool {
        self.inner
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .token
            .as_deref()
            == Some(candidate)
    }

    fn snapshot_json(&self) -> String {
        let inner = self.inner.lock().unwrap_or_else(|error| error.into_inner());
        serde_json::to_string(&inner.snapshot).unwrap_or_else(|_| "{}".to_string())
    }

    fn subscribe(&self) -> Option<broadcast::Receiver<String>> {
        self.inner
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .sender
            .as_ref()
            .map(broadcast::Sender::subscribe)
    }
}

#[tauri::command]
pub async fn start_companion_server(
    app: AppHandle,
    runtime: State<'_, CompanionRuntime>,
    token: String,
) -> Result<CompanionServerInfo, String> {
    let on_generate: GenerateCallback = Arc::new(move || {
        let _ = app.emit("companion-generate-requested", ());
    });
    runtime.start(token, on_generate).await
}

#[tauri::command]
pub fn stop_companion_server(
    runtime: State<'_, CompanionRuntime>,
) -> Result<CompanionServerInfo, String> {
    runtime.stop();
    runtime.info()
}

#[tauri::command]
pub fn get_companion_server_info(
    runtime: State<'_, CompanionRuntime>,
) -> Result<CompanionServerInfo, String> {
    runtime.info()
}

#[tauri::command]
pub fn update_companion_transcript(runtime: State<'_, CompanionRuntime>, text: String) {
    runtime.update_live_transcript(text.trim());
}

async fn handle_connection(
    mut stream: TcpStream,
    runtime: CompanionRuntime,
    on_generate: GenerateCallback,
) -> Result<(), String> {
    let mut buffer = vec![0_u8; MAX_REQUEST_BYTES];
    let size = stream
        .read(&mut buffer)
        .await
        .map_err(|error| error.to_string())?;
    if size == 0 {
        return Ok(());
    }
    let request = String::from_utf8_lossy(&buffer[..size]);
    let Some((method, target)) = request.lines().next().and_then(|line| {
        let mut parts = line.split_whitespace();
        Some((parts.next()?, parts.next()?))
    }) else {
        return write_status(&mut stream, "400 Bad Request", "请求格式错误").await;
    };

    if method == "POST" {
        let Some(token) = target.strip_prefix("/generate?token=") else {
            return write_status(&mut stream, "404 Not Found", "控制指令不存在").await;
        };
        if !runtime.token_matches(token) {
            return write_status(&mut stream, "403 Forbidden", "会话地址无效").await;
        }
        on_generate();
        return write_json(&mut stream, "202 Accepted", r#"{"accepted":true}"#).await;
    }

    if method != "GET" {
        return write_status(&mut stream, "405 Method Not Allowed", "请求方法不支持").await;
    }

    if let Some(token) = target.strip_prefix("/events?token=") {
        if !runtime.token_matches(token) {
            return write_status(&mut stream, "403 Forbidden", "会话地址无效").await;
        }
        return stream_events(stream, runtime).await;
    }

    let token = target.trim_start_matches('/').trim_end_matches('/');
    if !runtime.token_matches(token) {
        return write_status(&mut stream, "404 Not Found", "同步会话不存在").await;
    }
    write_html(&mut stream, COMPANION_PAGE).await
}

async fn stream_events(mut stream: TcpStream, runtime: CompanionRuntime) -> Result<(), String> {
    let headers = concat!(
        "HTTP/1.1 200 OK\r\n",
        "Content-Type: text/event-stream; charset=utf-8\r\n",
        "Cache-Control: no-store\r\n",
        "Connection: keep-alive\r\n",
        "X-Content-Type-Options: nosniff\r\n\r\n"
    );
    stream
        .write_all(headers.as_bytes())
        .await
        .map_err(|error| error.to_string())?;
    write_event(&mut stream, &runtime.snapshot_json()).await?;

    let Some(mut receiver) = runtime.subscribe() else {
        return Ok(());
    };
    let mut keep_alive = time::interval(KEEP_ALIVE_INTERVAL);
    loop {
        tokio::select! {
            message = receiver.recv() => {
                match message {
                    Ok(payload) => write_event(&mut stream, &payload).await?,
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        write_event(&mut stream, &runtime.snapshot_json()).await?;
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
            _ = keep_alive.tick() => {
                stream.write_all(b": keep-alive\n\n").await.map_err(|error| error.to_string())?;
            }
        }
    }
    Ok(())
}

async fn write_event(stream: &mut TcpStream, payload: &str) -> Result<(), String> {
    let event = format!("event: answer\ndata: {payload}\n\n");
    stream
        .write_all(event.as_bytes())
        .await
        .map_err(|error| error.to_string())
}

async fn write_html(stream: &mut TcpStream, body: &str) -> Result<(), String> {
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nContent-Security-Policy: default-src 'self'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; connect-src 'self'\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    stream
        .write_all(response.as_bytes())
        .await
        .map_err(|error| error.to_string())
}

async fn write_json(stream: &mut TcpStream, status: &str, body: &str) -> Result<(), String> {
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json; charset=utf-8\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    stream
        .write_all(response.as_bytes())
        .await
        .map_err(|error| error.to_string())
}

async fn write_status(stream: &mut TcpStream, status: &str, message: &str) -> Result<(), String> {
    let body =
        format!("<!doctype html><meta charset=utf-8><title>{status}</title><p>{message}</p>");
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    stream
        .write_all(response.as_bytes())
        .await
        .map_err(|error| error.to_string())
}

fn validate_token(token: &str) -> Result<(), String> {
    if (12..=64).contains(&token.len())
        && token
            .chars()
            .all(|character| character.is_ascii_alphanumeric())
    {
        Ok(())
    } else {
        Err("同步会话标识格式无效".to_string())
    }
}

fn local_ipv4() -> Option<Ipv4Addr> {
    #[cfg(target_os = "macos")]
    if let Some(address) = preferred_lan_ipv4(&macos_ipv4_interfaces()) {
        return Some(address);
    }

    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    socket.connect((Ipv4Addr::new(192, 0, 2, 1), 80)).ok()?;
    match socket.local_addr().ok()?.ip() {
        IpAddr::V4(address) if address.is_private() && !address.is_loopback() => Some(address),
        _ => None,
    }
}

fn preferred_lan_ipv4(candidates: &[(String, Ipv4Addr)]) -> Option<Ipv4Addr> {
    candidates
        .iter()
        .filter(|(name, address)| {
            address.is_private()
                && !address.is_loopback()
                && !address.is_link_local()
                && !name.starts_with("utun")
                && !name.starts_with("awdl")
                && !name.starts_with("llw")
                && !name.starts_with("ap")
        })
        .min_by_key(|(name, _)| {
            if name == "en0" {
                0
            } else if name.starts_with("en") {
                1
            } else if name.starts_with("bridge") {
                2
            } else {
                3
            }
        })
        .map(|(_, address)| *address)
}

#[cfg(target_os = "macos")]
fn macos_ipv4_interfaces() -> Vec<(String, Ipv4Addr)> {
    let mut head = std::ptr::null_mut::<libc::ifaddrs>();
    // SAFETY: getifaddrs initializes `head` on success. Every node is read-only here and the
    // complete list is released exactly once with freeifaddrs before this function returns.
    if unsafe { libc::getifaddrs(&mut head) } != 0 || head.is_null() {
        return Vec::new();
    }

    let mut interfaces = Vec::new();
    let mut current = head;
    while !current.is_null() {
        // SAFETY: `current` is part of the linked list returned by getifaddrs and remains valid
        // until freeifaddrs is called after traversal.
        let interface = unsafe { &*current };
        if !interface.ifa_addr.is_null()
            && unsafe { (*interface.ifa_addr).sa_family as i32 } == libc::AF_INET
            && interface.ifa_flags & libc::IFF_UP as u32 != 0
            && interface.ifa_flags & libc::IFF_LOOPBACK as u32 == 0
        {
            // SAFETY: AF_INET guarantees a sockaddr_in, and ifa_name is a null-terminated name
            // owned by the getifaddrs list for the duration of this traversal.
            let address = unsafe { &*(interface.ifa_addr as *const libc::sockaddr_in) };
            let name = unsafe { CStr::from_ptr(interface.ifa_name) }
                .to_string_lossy()
                .into_owned();
            interfaces.push((name, Ipv4Addr::from(address.sin_addr.s_addr.to_ne_bytes())));
        }
        current = interface.ifa_next;
    }

    // SAFETY: `head` is the unchanged pointer returned by a successful getifaddrs call.
    unsafe { libc::freeifaddrs(head) };
    interfaces
}

fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

const COMPANION_PAGE: &str = r#"<!doctype html>
<html lang="zh-CN">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width,initial-scale=1,viewport-fit=cover">
  <meta name="color-scheme" content="dark">
  <title>Interview Copilot Companion</title>
  <style>
    :root { font-family: -apple-system,BlinkMacSystemFont,"PingFang SC",sans-serif; color:#edf4fa; background:#080c11; }
    * { box-sizing:border-box; }
    body { margin:0; min-height:100dvh; padding:max(18px,env(safe-area-inset-top)) max(18px,env(safe-area-inset-right)) calc(92px + env(safe-area-inset-bottom)) max(18px,env(safe-area-inset-left)); background:radial-gradient(circle at 90% 0,rgba(42,201,151,.12),transparent 34%),#080c11; }
    header { display:flex; align-items:center; justify-content:space-between; gap:12px; margin-bottom:18px; }
    h1 { margin:0; font-size:17px; }
    #state { color:#75dcb5; font-size:12px; }
    section { margin-bottom:14px; padding:16px; border:1px solid #24303b; border-radius:15px; background:rgba(15,21,28,.94); }
    label { display:block; margin-bottom:8px; color:#788797; font-size:11px; font-weight:700; letter-spacing:.08em; }
    #live-transcript { color:#edf4fa; font-size:16px; line-height:1.7; white-space:pre-wrap; }
    #question { color:#aebbc7; font-size:14px; line-height:1.65; white-space:pre-wrap; }
    #answer { min-height:35dvh; color:#f1f6fa; font-size:18px; line-height:1.8; white-space:pre-wrap; }
    .empty { color:#667381 !important; }
    #generate { position:fixed; z-index:10; right:max(18px,env(safe-area-inset-right)); bottom:max(14px,env(safe-area-inset-bottom)); left:max(18px,env(safe-area-inset-left)); min-height:58px; border:1px solid #51d5a3; border-radius:16px; color:#06120e; background:#6ce1b4; box-shadow:0 12px 32px rgba(0,0,0,.38); font:700 17px/1 -apple-system,BlinkMacSystemFont,"PingFang SC",sans-serif; }
    #generate:active { transform:translateY(1px); }
    #generate:disabled { opacity:.58; }
    footer { color:#586675; font-size:10px; text-align:center; }
  </style>
</head>
<body>
  <header><h1>Interview Copilot</h1><span id="state">正在连接…</span></header>
  <section><label>面试官实时转写</label><div id="live-transcript" class="empty">等待面试官说话</div></section>
  <section><label>本次生成所用问题</label><div id="question" class="empty">点击生成建议后固定问题</div></section>
  <section><label>AI 回答建议</label><div id="answer" class="empty">等待生成回答</div></section>
  <footer>局域网临时会话 · 关闭 Mac 同步后立即失效</footer>
  <button id="generate" type="button">生成建议</button>
  <script>
    const token = location.pathname.split('/').filter(Boolean)[0] || '';
    const state = document.querySelector('#state');
    const liveTranscript = document.querySelector('#live-transcript');
    const question = document.querySelector('#question');
    const answer = document.querySelector('#answer');
    const generate = document.querySelector('#generate');
    let requestPending = false;
    let requestTimer;

    function updateGenerateButton(generating) {
      generate.disabled = generating || requestPending;
      generate.textContent = generating ? '正在生成…' : (requestPending ? '正在请求…' : '生成建议');
    }

    generate.addEventListener('click', async () => {
      if (generate.disabled) return;
      requestPending = true;
      updateGenerateButton(false);
      state.textContent = '正在通知 Mac…';
      try {
        const response = await fetch('/generate?token=' + encodeURIComponent(token), {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: '{}'
        });
        if (!response.ok) throw new Error('request rejected');
        state.textContent = '已请求生成回答';
        clearTimeout(requestTimer);
        requestTimer = setTimeout(() => {
          requestPending = false;
          updateGenerateButton(false);
        }, 3000);
      } catch (_) {
        requestPending = false;
        state.textContent = '请求失败，请检查 Mac 连接';
        updateGenerateButton(false);
      }
    });

    const source = new EventSource('/events?token=' + encodeURIComponent(token));
    source.addEventListener('open', () => { state.textContent = '已连接'; });
    source.addEventListener('error', () => { state.textContent = '正在重连…'; });
    source.addEventListener('answer', (event) => {
      const data = JSON.parse(event.data);
      liveTranscript.textContent = data.liveTranscript || '等待面试官说话';
      question.textContent = data.question || '点击生成建议后固定问题';
      answer.textContent = data.answer || (data.status || '等待生成回答');
      liveTranscript.classList.toggle('empty', !data.liveTranscript);
      question.classList.toggle('empty', !data.question);
      answer.classList.toggle('empty', !data.answer);
      state.textContent = data.status || '已连接';
      const generating = data.status === '正在生成回答';
      if (generating || data.done) {
        requestPending = false;
        clearTimeout(requestTimer);
      }
      updateGenerateButton(generating);
    });
  </script>
</body>
</html>"#;

#[cfg(test)]
mod tests {
    use super::{
        preferred_lan_ipv4, validate_token, CompanionRuntime, GenerateCallback, COMPANION_PAGE,
    };
    use futures_util::StreamExt;
    use std::{
        net::Ipv4Addr,
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        },
    };
    use tokio::time::{timeout, Duration};

    #[test]
    fn accepts_browser_generated_session_tokens() {
        assert!(validate_token("a94F20cdd3104ba9").is_ok());
    }

    #[test]
    fn rejects_short_or_punctuated_session_tokens() {
        assert!(validate_token("short").is_err());
        assert!(validate_token("not-safe-token!").is_err());
    }

    #[test]
    fn prefers_wifi_and_ignores_vpn_interfaces() {
        let candidates = vec![
            ("utun4".to_string(), Ipv4Addr::new(198, 18, 0, 1)),
            ("bridge0".to_string(), Ipv4Addr::new(192, 168, 64, 1)),
            ("en0".to_string(), Ipv4Addr::new(192, 168, 0, 216)),
        ];
        assert_eq!(
            preferred_lan_ipv4(&candidates),
            Some(Ipv4Addr::new(192, 168, 0, 216))
        );
    }

    #[test]
    fn companion_page_has_separate_live_and_fixed_question_sections() {
        assert!(COMPANION_PAGE.contains("面试官实时转写"));
        assert!(COMPANION_PAGE.contains("本次生成所用问题"));
        assert!(COMPANION_PAGE.contains("data.liveTranscript"));
    }

    #[tokio::test]
    #[ignore = "requires permission to bind a loopback TCP listener"]
    async fn serves_the_companion_page_and_current_answer_snapshot() {
        let runtime = CompanionRuntime::default();
        let token = "TestSession1234";
        let generate_count = Arc::new(AtomicUsize::new(0));
        let callback_count = generate_count.clone();
        let on_generate: GenerateCallback = Arc::new(move || {
            callback_count.fetch_add(1, Ordering::SeqCst);
        });
        let info = runtime.start(token.to_string(), on_generate).await.unwrap();
        let client = reqwest::Client::new();
        let base_url = format!("http://127.0.0.1:{}", info.port);

        let page = client
            .get(format!("{base_url}/{token}"))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert!(page.contains("Interview Copilot Companion"));
        assert!(page.contains("生成建议"));

        let control_response = client
            .post(format!("{base_url}/generate?token={token}"))
            .json(&serde_json::json!({}))
            .send()
            .await
            .unwrap();
        assert_eq!(control_response.status(), reqwest::StatusCode::ACCEPTED);
        assert_eq!(generate_count.load(Ordering::SeqCst), 1);

        runtime.update_live_transcript("请介绍一下你的项目");
        runtime.begin_answer("request-1", "请介绍一下项目", "zh");
        runtime.apply_stream("request-1", "这是回答。", true, None);
        let response = client
            .get(format!("{base_url}/events?token={token}"))
            .send()
            .await
            .unwrap();
        let mut stream = response.bytes_stream();
        let chunk = timeout(Duration::from_secs(2), stream.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let event = String::from_utf8_lossy(&chunk);
        assert!(event.contains("请介绍一下你的项目"));
        assert!(event.contains("请介绍一下项目"));
        assert!(event.contains("这是回答。"));

        runtime.stop();
    }
}
