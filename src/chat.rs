//! Live chat from Twitch, Kick and YouTube in one feed.
//!
//! - **Twitch:** the public IRC chat over WebSocket, read anonymously.
//! - **Kick:** the channel's chat room over its public Pusher socket.
//! - **YouTube:** the official Data API (your own API key), polling the
//!   live chat of a video or of a channel's current live stream.
//!
//! Each runs on its own thread and reconnects on its own; messages land in
//! a shared feed the app reads every frame.

use std::collections::VecDeque;
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket};

/// Where a message came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Platform {
    Twitch,
    Kick,
    YouTube,
}

impl Platform {
    pub fn label(self) -> &'static str {
        match self {
            Platform::Twitch => "Twitch",
            Platform::Kick => "Kick",
            Platform::YouTube => "YouTube",
        }
    }
    pub fn color(self) -> [u8; 3] {
        match self {
            Platform::Twitch => [145, 70, 255],
            Platform::Kick => [83, 252, 24],
            Platform::YouTube => [255, 48, 48],
        }
    }
}

/// One chat message.
#[derive(Clone, Debug, PartialEq)]
pub struct Msg {
    pub id: u64,
    pub platform: Platform,
    pub author: String,
    pub text: String,
    /// The author's chosen color, when the platform has one.
    pub color: Option<[u8; 3]>,
}

/// What to connect to. Empty fields are skipped.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Twitch channel name.
    pub twitch: String,
    /// Kick channel name.
    pub kick: String,
    /// YouTube: a live video's link or ID, a channel ID (UC…) or @handle.
    pub youtube: String,
}

impl Config {
    pub fn any(&self) -> bool {
        !self.twitch.trim().is_empty() || !self.kick.trim().is_empty() || !self.youtube.trim().is_empty()
    }
}

/// The shared feed: newest last, and each platform's state.
#[derive(Default)]
pub struct Feed {
    pub msgs: VecDeque<Msg>,
    pub states: Vec<(Platform, String)>,
    next: u64,
}

const KEEP: usize = 300;

impl Feed {
    fn push(&mut self, platform: Platform, author: String, text: String, color: Option<[u8; 3]>) {
        self.next += 1;
        self.msgs.push_back(Msg { id: self.next, platform, author, text, color });
        while self.msgs.len() > KEEP {
            self.msgs.pop_front();
        }
    }
    fn state(&mut self, platform: Platform, s: impl Into<String>) {
        let s = s.into();
        match self.states.iter_mut().find(|(p, _)| *p == platform) {
            Some(slot) => slot.1 = s,
            None => self.states.push((platform, s)),
        }
    }
}

/// The running connections.
pub struct Chat {
    pub feed: Arc<Mutex<Feed>>,
    stop: Arc<AtomicBool>,
    pub config: Config,
}

impl Drop for Chat {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

impl Chat {
    /// Connect to every platform set in `config`. `yt_key`: your YouTube
    /// Data API key.
    pub fn start(config: Config, yt_key: Option<String>) -> Chat {
        let feed = Arc::new(Mutex::new(Feed::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let spawn = |platform: Platform, f: Box<dyn Fn(&Arc<Mutex<Feed>>, &AtomicBool) -> Result<()> + Send>| {
            let (feed, stop) = (feed.clone(), stop.clone());
            let _ = std::thread::Builder::new().name(format!("chat-{}", platform.label())).spawn(move || {
                // Reconnect until stopped, waiting longer after each failure.
                let mut wait = 2u64;
                while !stop.load(Ordering::Relaxed) {
                    feed.lock().unwrap().state(platform, crate::i18n::l("connecting…", "conectando…"));
                    let t = Instant::now();
                    match f(&feed, &stop) {
                        Ok(()) => wait = 2,
                        Err(e) => {
                            feed.lock().unwrap().state(platform, format!("{e:#}"));
                            if t.elapsed() > Duration::from_secs(60) {
                                wait = 2;
                            }
                        }
                    }
                    for _ in 0..wait * 10 {
                        if stop.load(Ordering::Relaxed) {
                            return;
                        }
                        std::thread::sleep(Duration::from_millis(100));
                    }
                    wait = (wait * 2).min(60);
                }
            });
        };
        let t = config.twitch.trim().trim_start_matches('#').to_lowercase();
        if !t.is_empty() {
            spawn(Platform::Twitch, Box::new(move |feed, stop| twitch(&t, feed, stop)));
        }
        let k = config.kick.trim().to_lowercase();
        if !k.is_empty() {
            spawn(Platform::Kick, Box::new(move |feed, stop| kick(&k, feed, stop)));
        }
        let y = config.youtube.trim().to_string();
        if !y.is_empty() {
            let key = yt_key.unwrap_or_default();
            spawn(Platform::YouTube, Box::new(move |feed, stop| youtube(&y, &key, feed, stop)));
        }
        Chat { feed, stop, config }
    }
}

fn connect(url: &str) -> Result<WebSocket<MaybeTlsStream<TcpStream>>> {
    let (ws, _) = tungstenite::connect(url).with_context(|| format!("connecting to {}", url.split('?').next().unwrap_or(url)))?;
    // Short reads so the thread notices when it's stopped.
    let tcp = match ws.get_ref() {
        MaybeTlsStream::Plain(s) => s,
        MaybeTlsStream::NativeTls(s) => s.get_ref(),
        _ => bail!("unexpected socket"),
    };
    tcp.set_read_timeout(Some(Duration::from_millis(500)))?;
    Ok(ws)
}

/// Read one message, or None on a read timeout.
fn read(ws: &mut WebSocket<MaybeTlsStream<TcpStream>>) -> Result<Option<Message>> {
    match ws.read() {
        Ok(m) => Ok(Some(m)),
        Err(tungstenite::Error::Io(e)) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

fn hex_color(s: &str) -> Option<[u8; 3]> {
    let s = s.trim().trim_start_matches('#');
    if s.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(s, 16).ok()?;
    Some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

// ---- Twitch ----

/// A Twitch IRC line → (display name, color, text) for chat messages.
pub fn parse_twitch(line: &str) -> Option<(String, Option<[u8; 3]>, String)> {
    let (tags, rest) = match line.strip_prefix('@') {
        Some(l) => l.split_once(' ')?,
        None => ("", line),
    };
    // ":nick!nick@host.example.com PRIVMSG #chan :text"
    let rest = rest.strip_prefix(':')?;
    let (prefix, rest) = rest.split_once(' ')?;
    let rest = rest.strip_prefix("PRIVMSG ")?;
    let (_, text) = rest.split_once(" :")?;
    let tag = |k: &str| tags.split(';').find_map(|t| t.strip_prefix(k).and_then(|v| v.strip_prefix('='))).filter(|v| !v.is_empty());
    let name = tag("display-name").map(str::to_string).unwrap_or_else(|| prefix.split('!').next().unwrap_or("").to_string());
    let text = text.strip_prefix("\u{1}ACTION ").map(|t| t.trim_end_matches('\u{1}')).unwrap_or(text);
    Some((name, tag("color").and_then(hex_color), text.to_string()))
}

fn twitch(channel: &str, feed: &Arc<Mutex<Feed>>, stop: &AtomicBool) -> Result<()> {
    let mut ws = connect("wss://irc-ws.chat.twitch.tv:443")?;
    let nick = format!("justinfan{}", 10000 + std::process::id() % 80000);
    for line in ["CAP REQ :twitch.tv/tags".to_string(), "PASS SCHMOOPIIE".into(), format!("NICK {nick}"), format!("JOIN #{channel}")] {
        ws.send(Message::text(line))?;
    }
    feed.lock().unwrap().state(Platform::Twitch, format!("#{channel}"));
    while !stop.load(Ordering::Relaxed) {
        let Some(m) = read(&mut ws)? else { continue };
        let Message::Text(t) = m else { continue };
        for line in t.as_str().split("\r\n").filter(|l| !l.is_empty()) {
            if let Some(p) = line.strip_prefix("PING ") {
                ws.send(Message::text(format!("PONG {p}")))?;
            } else if let Some((name, color, text)) = parse_twitch(line) {
                feed.lock().unwrap().push(Platform::Twitch, name, text, color);
            } else if line.contains(" NOTICE ") && line.contains("Login") {
                bail!("{line}");
            }
        }
    }
    Ok(())
}

// ---- Kick ----

/// A Kick chat event's data (a JSON string) → (name, color, text).
pub fn parse_kick(data: &str) -> Option<(String, Option<[u8; 3]>, String)> {
    let v: serde_json::Value = serde_json::from_str(data).ok()?;
    let name = v["sender"]["username"].as_str()?.to_string();
    let color = v["sender"]["identity"]["color"].as_str().and_then(hex_color);
    // Emotes arrive as "[emote:123:name]": keep the name.
    let raw = v["content"].as_str()?;
    let mut text = String::new();
    let mut rest = raw;
    while let Some(i) = rest.find("[emote:") {
        text.push_str(&rest[..i]);
        let after = &rest[i + 7..];
        match after.find(']') {
            Some(j) => {
                text.push_str(after[..j].rsplit(':').next().unwrap_or(""));
                rest = &after[j + 1..];
            }
            None => {
                rest = &rest[i..];
                break;
            }
        }
    }
    text.push_str(rest);
    Some((name, color, text))
}

/// Kick's chat socket: (Pusher app key, cluster), from its realtime API.
fn kick_socket() -> Result<(String, String)> {
    let body = serde_json::json!({ "client": { "id": format!("chisme-{}", std::process::id()), "type": "web" }, "capabilities": { "accepted_providers": [{ "provider": "pusher" }] } });
    let text = ureq::post("https://web.kick.com/api/v1/realtime/connection")
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) Chisme")
        .header("Accept", "application/json")
        .header("Content-Type", "application/json")
        .send(body.to_string())
        .context("Kick's chat service")?
        .body_mut()
        .read_to_string()?;
    let v: serde_json::Value = serde_json::from_str(&text).context("Kick's chat service answer")?;
    let c = v["data"]["connections"].as_array().and_then(|a| a.iter().find(|c| c["provider"] == "pusher")).ok_or_else(|| anyhow!("Kick's chat service offered no connection"))?;
    let key = c["credentials"]["app_key"].as_str().ok_or_else(|| anyhow!("no Kick chat key"))?;
    let cluster = c["credentials"]["cluster"].as_str().unwrap_or("us2");
    Ok((key.to_string(), cluster.to_string()))
}

fn kick(channel: &str, feed: &Arc<Mutex<Feed>>, stop: &AtomicBool) -> Result<()> {
    let body = ureq::get(&format!("https://kick.com/api/v2/channels/{channel}"))
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) Chisme")
        .header("Accept", "application/json")
        .call()
        .map_err(|e| anyhow!(crate::t!("Kick didn't answer for \"{}\" ({})", "Kick no respondió para \"{}\" ({})", channel, e)))?
        .body_mut()
        .read_to_string()?;
    let v: serde_json::Value = serde_json::from_str(&body).context("Kick's answer")?;
    let room = v["chatroom"]["id"].as_u64().ok_or_else(|| anyhow!(crate::t!("no Kick channel \"{}\"", "no existe el canal de Kick \"{}\"", channel)))?;
    // Kick hands out its socket key and cluster per connection.
    let (key, cluster) = kick_socket()?;
    let mut ws = connect(&format!("wss://ws-{cluster}.pusher.com/app/{key}?protocol=7&client=js&version=8.4.0&flash=false"))?;
    let sub = serde_json::json!({ "event": "pusher:subscribe", "data": { "auth": "", "channel": format!("chatrooms.{room}.v2") } });
    ws.send(Message::text(sub.to_string()))?;
    feed.lock().unwrap().state(Platform::Kick, channel.to_string());
    let mut last_ping = Instant::now();
    while !stop.load(Ordering::Relaxed) {
        if last_ping.elapsed() > Duration::from_secs(60) {
            ws.send(Message::text(r#"{"event":"pusher:ping","data":{}}"#))?;
            last_ping = Instant::now();
        }
        let Some(m) = read(&mut ws)? else { continue };
        let Message::Text(t) = m else { continue };
        let Ok(ev) = serde_json::from_str::<serde_json::Value>(t.as_str()) else { continue };
        match ev["event"].as_str() {
            Some("pusher:ping") => ws.send(Message::text(r#"{"event":"pusher:pong","data":{}}"#))?,
            Some("App\\Events\\ChatMessageEvent") => {
                if let Some((name, color, text)) = ev["data"].as_str().and_then(parse_kick) {
                    feed.lock().unwrap().push(Platform::Kick, name, text, color);
                }
            }
            _ => {}
        }
    }
    Ok(())
}

// ---- YouTube ----

/// What the YouTube field holds.
#[derive(Debug, PartialEq)]
pub enum YtTarget {
    Video(String),
    Channel(String),
    Handle(String),
}

pub fn parse_youtube(input: &str) -> Option<YtTarget> {
    let s = input.trim();
    if s.is_empty() {
        return None;
    }
    if let Some(h) = s.split('/').find(|p| p.starts_with('@')) {
        return Some(YtTarget::Handle(h.to_string()));
    }
    if let Some(i) = s.find("v=") {
        return Some(YtTarget::Video(s[i + 2..].split(['&', '#']).next()?.to_string()));
    }
    for marker in ["youtu.be/", "/live/", "/shorts/", "/embed/"] {
        if let Some(i) = s.find(marker) {
            return Some(YtTarget::Video(s[i + marker.len()..].split(['?', '&', '/', '#']).next()?.to_string()));
        }
    }
    if let Some(i) = s.find("/channel/") {
        return Some(YtTarget::Channel(s[i + 9..].split(['?', '/']).next()?.to_string()));
    }
    if s.starts_with("UC") && s.len() == 24 {
        return Some(YtTarget::Channel(s.to_string()));
    }
    (s.len() == 11 && !s.contains(' ')).then(|| YtTarget::Video(s.to_string()))
}

fn yt_get(path: &str, key: &str) -> Result<serde_json::Value> {
    let url = format!("https://www.googleapis.com/youtube/v3/{path}&key={key}");
    let mut resp = ureq::get(&url).config().http_status_as_error(false).build().call()?;
    let status = resp.status();
    let v: serde_json::Value = serde_json::from_str(&resp.body_mut().read_to_string()?).context("YouTube's answer")?;
    if !status.is_success() {
        let msg = v["error"]["message"].as_str().unwrap_or("error");
        bail!("YouTube: {msg}");
    }
    Ok(v)
}

fn youtube(input: &str, key: &str, feed: &Arc<Mutex<Feed>>, stop: &AtomicBool) -> Result<()> {
    if key.trim().is_empty() {
        bail!(crate::i18n::l("add your YouTube API key", "agrega tu llave de la API de YouTube"));
    }
    let target = parse_youtube(input).ok_or_else(|| anyhow!(crate::i18n::l("paste the live video link or the channel", "pega el link del en vivo o del canal")))?;
    let channel = match &target {
        YtTarget::Video(_) => None,
        YtTarget::Channel(c) => Some(c.clone()),
        YtTarget::Handle(h) => {
            let v = yt_get(&format!("channels?part=id&forHandle={h}"), key)?;
            Some(v["items"][0]["id"].as_str().ok_or_else(|| anyhow!(crate::t!("no channel {}", "no existe el canal {}", h)))?.to_string())
        }
    };
    let video = match (&target, channel) {
        (YtTarget::Video(v), _) => v.clone(),
        (_, Some(c)) => {
            let v = yt_get(&format!("search?part=id&channelId={c}&eventType=live&type=video&maxResults=1"), key)?;
            v["items"][0]["id"]["videoId"].as_str().ok_or_else(|| anyhow!(crate::i18n::l("the channel isn't live yet", "el canal aún no está en vivo")))?.to_string()
        }
        _ => unreachable!(),
    };
    let v = yt_get(&format!("videos?part=liveStreamingDetails&id={video}"), key)?;
    let chat = v["items"][0]["liveStreamingDetails"]["activeLiveChatId"].as_str().ok_or_else(|| anyhow!(crate::i18n::l("that video has no live chat now", "ese video no tiene chat en vivo ahora")))?.to_string();
    feed.lock().unwrap().state(Platform::YouTube, video.clone());
    let mut token = String::new();
    let mut first = true;
    while !stop.load(Ordering::Relaxed) {
        let page = if token.is_empty() { String::new() } else { format!("&pageToken={token}") };
        let v = yt_get(&format!("liveChat/messages?liveChatId={chat}&part=snippet,authorDetails&maxResults=200{page}"), key)?;
        let items = v["items"].as_array().cloned().unwrap_or_default();
        // On the first page, only the last few (not the whole history).
        let skip = if first { items.len().saturating_sub(10) } else { 0 };
        first = false;
        {
            let mut f = feed.lock().unwrap();
            for it in items.iter().skip(skip) {
                let text = it["snippet"]["displayMessage"].as_str().unwrap_or("").to_string();
                let name = it["authorDetails"]["displayName"].as_str().unwrap_or("").to_string();
                if !text.is_empty() {
                    f.push(Platform::YouTube, name, text, None);
                }
            }
        }
        token = v["nextPageToken"].as_str().unwrap_or("").to_string();
        // At least 5 s between polls keeps a day's quota for hours of chat.
        let wait = v["pollingIntervalMillis"].as_u64().unwrap_or(5000).max(5000);
        let until = Instant::now() + Duration::from_millis(wait);
        while Instant::now() < until && !stop.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(200));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn twitch_lines() {
        let l = "@badge-info=;color=#1E90FF;display-name=JorgeFan;emotes= :jorgefan!jorgefan@host.example.com PRIVMSG #lordvayron :hola chat";
        assert_eq!(parse_twitch(l), Some(("JorgeFan".into(), Some([0x1e, 0x90, 0xff]), "hola chat".into())));
        assert_eq!(parse_twitch(":tmi.twitch.tv 001 justinfan1 :Welcome"), None);
        let me = "@color=;display-name=X :x!x@host.example.com PRIVMSG #c :\u{1}ACTION waves\u{1}";
        assert_eq!(parse_twitch(me).unwrap().2, "waves");
    }

    #[test]
    fn kick_messages() {
        let d = r##"{"id":"1","content":"gg [emote:37226:KEKW] wp","sender":{"username":"ana","identity":{"color":"#FF5500"}}}"##;
        assert_eq!(parse_kick(d), Some(("ana".into(), Some([255, 85, 0]), "gg KEKW wp".into())));
    }

    #[test]
    fn youtube_inputs() {
        assert_eq!(parse_youtube("https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=1"), Some(YtTarget::Video("dQw4w9WgXcQ".into())));
        assert_eq!(parse_youtube("https://youtube.com/live/abcdefghijk?si=x"), Some(YtTarget::Video("abcdefghijk".into())));
        assert_eq!(parse_youtube("https://youtu.be/abcdefghijk"), Some(YtTarget::Video("abcdefghijk".into())));
        assert_eq!(parse_youtube("@LordVayron"), Some(YtTarget::Handle("@LordVayron".into())));
        assert_eq!(parse_youtube("https://www.youtube.com/@LordVayron/live"), Some(YtTarget::Handle("@LordVayron".into())));
        assert_eq!(parse_youtube("UCabcdefghijklmnopqrstuv"), Some(YtTarget::Channel("UCabcdefghijklmnopqrstuv".into())));
    }
}

#[cfg(test)]
mod live_probe {
    use super::*;

    /// T=twitch_channel K=kick_channel: print a few live messages.
    #[test]
    #[ignore]
    fn live_chat_probe() {
        let c = Chat::start(Config { twitch: std::env::var("T").unwrap_or_default(), kick: std::env::var("K").unwrap_or_default(), youtube: String::new() }, None);
        for _ in 0..45 {
            std::thread::sleep(Duration::from_secs(1));
            if c.feed.lock().unwrap().msgs.len() >= 4 {
                break;
            }
        }
        let f = c.feed.lock().unwrap();
        println!("states {:?}", f.states);
        for m in f.msgs.iter().take(6) {
            println!("{:?} {}: {}", m.platform, m.author, m.text.chars().take(60).collect::<String>());
        }
    }
}
