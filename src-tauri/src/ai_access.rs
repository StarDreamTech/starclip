//! Session-only access: read-only records and separately authorized pet signals.
//! The desktop owns the Store and holds its locks
//! until the response is written; adapters never open SQLite or read clipboard.
use crate::store::{Clip, Result, Store};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{BufRead, Write},
    net::{SocketAddr, TcpStream},
    path::Path,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub const DENIED: &str = "资料不可用或未获授权。";
pub const MAX_REQUEST: usize = 65_536;
pub const MAX_RESPONSE: usize = 1_048_576;
const MAX_RECORDS: usize = 500;
const MAX_RESULTS: usize = 50;
const MAX_HOURS: u64 = 168;
const FIELDS: [&str; 4] = ["title", "content", "tags", "sources"];
pub const SKILL: &str = include_str!("../../docs/ai-skill/SKILL.md");

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
fn random_id() -> Result<String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|_| "无法生成连接凭证。")?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
fn limited(value: &str, max: usize) -> String {
    value.chars().take(max).collect()
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Connection {
    pub id: String,
    pub name: String,
    pub record_count: usize,
    pub fields: Vec<String>,
    pub max_results: usize,
    pub expires_at: u64,
    pub allow_pet_control: bool,
    #[serde(skip)]
    last_pet_signal: Option<Instant>,
    #[serde(skip)]
    token: String,
    #[serde(skip)]
    ids: BTreeMap<i64, String>,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Audit {
    connection_name: String,
    action: String,
    at: u64,
    count: usize,
    result: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateConnection {
    pub name: String,
    pub clip_ids: Vec<i64>,
    pub fields: Vec<String>,
    pub max_results: usize,
    pub expires_hours: u64,
    #[serde(default)]
    pub allow_pet_control: bool,
}
#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PetSnapshot {
    pub revision: u64,
    pub signal: Option<PetSignal>,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PetSignal {
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    pub expires_at: u64,
    #[serde(skip)]
    owner: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PetArguments {
    status: Option<String>,
    action: Option<String>,
    message: Option<String>,
    duration_seconds: Option<u64>,
}
#[derive(Default)]
pub struct AiAccess {
    pub enabled: bool,
    pub port: Option<u16>,
    connections: Vec<Connection>,
    audit: Vec<Audit>,
    pet: PetSnapshot,
}

fn safe(clip: &Clip, revision: u32) -> bool {
    revision > 0
        && clip.scan_version == revision
        && !clip.is_private
        && !matches!(clip.kind.as_str(), "image" | "file")
}
impl AiAccess {
    pub fn state(&self) -> Value {
        json!({"enabled":self.enabled,"available":self.port.is_some(),"connections":self.connections.iter().filter(|c|c.expires_at>now()).collect::<Vec<_>>(),"audit":self.audit,"limits":{"maxRecords":MAX_RECORDS,"maxResults":MAX_RESULTS,"maxHours":MAX_HOURS}})
    }
    pub fn enable(&mut self, enabled: bool) -> Result<Value> {
        if enabled && self.port.is_none() {
            return Err("本地连接服务不可用，请重启星笺。".into());
        }
        self.enabled = enabled;
        if !enabled {
            self.connections.clear();
            self.clear_pet();
        }
        Ok(self.state())
    }
    pub fn revoke(&mut self, id: &str) -> Value {
        self.connections.retain(|c| c.id != id);
        if self.pet.signal.as_ref().is_some_and(|s| s.owner == id) {
            self.clear_pet();
        }
        self.state()
    }
    pub fn clear_audit(&mut self) -> Value {
        self.audit.clear();
        self.state()
    }
    pub fn create(
        &mut self,
        store: &Store,
        request: CreateConnection,
        executable: &Path,
    ) -> Result<Value> {
        if !self.enabled || self.port.is_none() {
            return Err("请先开启 AI 连接。".into());
        }
        if request.name.trim().is_empty()
            || request.name.chars().count() > 60
            || request.name.chars().any(char::is_control)
            || (request.clip_ids.is_empty() && !request.allow_pet_control)
            || request.clip_ids.len() > MAX_RECORDS
            || (request.clip_ids.is_empty() != request.fields.is_empty())
            || request.fields.len() > 4
            || request.fields.iter().any(|f| !FIELDS.contains(&f.as_str()))
            || !(1..=MAX_RESULTS).contains(&request.max_results)
            || !(1..=MAX_HOURS).contains(&request.expires_hours)
        {
            return Err("连接名称、记录范围、字段或额度无效。".into());
        }
        self.connections.retain(|c| c.expires_at > now());
        if self.connections.len() >= 20 {
            return Err("最多同时保留 20 个连接，请先撤销旧连接。".into());
        }
        let revision = store.privacy_rules()?.revision;
        if !request.clip_ids.is_empty() && store.has_pending_ai_history()? {
            return Err("请先在规则与隐私中完成历史扫描，再创建连接。".into());
        }
        let ids: BTreeSet<_> = request.clip_ids.into_iter().collect();
        let mut records = BTreeMap::new();
        for id in &ids {
            let clip = store.clip(*id).map_err(|_| DENIED)?;
            if !safe(&clip, revision) || clip.record_identity.is_empty() {
                return Err("所选记录含隐私、待检测、图片或不可用资料，请重新选择。".into());
            }
            records.insert(*id, clip.record_identity);
        }
        let connection = Connection {
            id: random_id()?,
            token: random_id()?,
            name: request.name.trim().into(),
            record_count: ids.len(),
            ids: records,
            fields: request.fields,
            max_results: request.max_results,
            expires_at: now() + request.expires_hours * 3_600_000,
            allow_pet_control: request.allow_pet_control,
            last_pet_signal: None,
        };
        let id = connection.id.clone();
        self.connections.push(connection);
        Ok(json!({"state":self.state(),"connectionId":id,"config":self.config(&id,executable)?}))
    }
    pub fn config(&self, id: &str, executable: &Path) -> Result<Value> {
        let c = self
            .connections
            .iter()
            .find(|c| c.id == id && c.expires_at > now())
            .ok_or(DENIED)?;
        if !self.enabled {
            return Err(DENIED.into());
        }
        let port = self.port.ok_or(DENIED)?;
        let path = executable.to_str().ok_or("应用程序路径无效。")?;
        #[cfg(windows)]
        let cli = format!(
            "$env:STARCLIP_AI_TOKEN = '{}'; & '{}' ai --port {} status",
            c.token,
            path.replace('\'', "''"),
            port
        );
        #[cfg(not(windows))]
        let cli = format!(
            "STARCLIP_AI_TOKEN='{}' '{}' ai --port {} status",
            c.token,
            path.replace('\'', "'\\''"),
            port
        );
        Ok(
            json!({"mcp":{"mcpServers":{"starclip":{"command":path,"args":["mcp","--port",port.to_string()],"env":{"STARCLIP_AI_TOKEN":c.token}}}},"cli":cli,"skill":SKILL}),
        )
    }
    /// Called while the desktop storage, store and access mutexes are held. Caller
    /// retains all guards through socket write, linearizing revocation and reads.
    pub fn respond(&mut self, store: &Store, request: Request) -> Value {
        self.respond_with_pet(store, request, |_| Ok(()))
    }
    pub fn pet_snapshot(&self) -> PetSnapshot {
        let mut snapshot = self.pet.clone();
        snapshot.signal = snapshot.signal.filter(|signal| {
            self.enabled
                && signal.expires_at > now()
                && self
                    .connections
                    .iter()
                    .any(|c| c.id == signal.owner && c.allow_pet_control && c.expires_at > now())
        });
        snapshot
    }
    /// Local user controls may clear a signal without revoking its connection.
    pub fn clear_pet(&mut self) {
        if self.pet.signal.take().is_some() {
            self.pet.revision = self.pet.revision.saturating_add(1);
        }
    }
    pub fn respond_with_pet(
        &mut self,
        store: &Store,
        request: Request,
        mut emit: impl FnMut(&PetSnapshot) -> Result<()>,
    ) -> Value {
        let c = self
            .connections
            .iter()
            .find(|c| self.enabled && c.token == request.token && c.expires_at > now())
            .cloned();
        let Some(c) = c else {
            return json!({"error":DENIED});
        };
        let action = if TOOLS.contains(&request.action.as_str()) {
            request.action.clone()
        } else {
            "invalid".into()
        };
        let result = if matches!(request.action.as_str(), "pet_signal" | "pet_clear") {
            self.pet_request(store, &c, &request.action, request.arguments, &mut emit)
        } else {
            self.query(store, &c, &request.action, request.arguments)
        };
        let (response, count, outcome) = match result {
            Ok((v, n)) => (json!({"result":v}), n, "ok"),
            Err(_) => (json!({"error":DENIED}), 0, "denied"),
        };
        self.audit.push(Audit {
            connection_name: c.name,
            action,
            at: now(),
            count,
            result: outcome.into(),
        });
        if self.audit.len() > 200 {
            self.audit.drain(..self.audit.len() - 200);
        }
        response
    }
    fn pet_request(
        &mut self,
        store: &Store,
        c: &Connection,
        action: &str,
        args: Value,
        emit: &mut impl FnMut(&PetSnapshot) -> Result<()>,
    ) -> Result<(Value, usize)> {
        if !c.allow_pet_control || !store.starpet_preferences()?.enabled {
            return Err(DENIED.into());
        }
        let signal = if action == "pet_clear" {
            if !args.as_object().is_some_and(|a| a.is_empty()) {
                return Err(DENIED.into());
            }
            if !self.pet.signal.as_ref().is_some_and(|s| s.owner == c.id) {
                return Ok((json!({"cleared":false}), 0));
            }
            None
        } else {
            let args: PetArguments = serde_json::from_value(args).map_err(|_| DENIED)?;
            let duration = args.duration_seconds.unwrap_or(10);
            if !(1..=60).contains(&duration)
                || args.status.as_ref().is_some_and(|s| {
                    !["idle", "thinking", "working", "waiting", "success", "error"]
                        .contains(&s.as_str())
                })
                || args
                    .message
                    .as_ref()
                    .is_some_and(|s| s.chars().count() > 120 || s.chars().any(char::is_control))
                || (args.status.is_none()
                    && args.action.is_none()
                    && args.message.as_ref().is_none_or(|s| s.trim().is_empty()))
                || c.last_pet_signal
                    .is_some_and(|last| last.elapsed() < Duration::from_millis(500))
            {
                return Err(DENIED.into());
            }
            if let Some(action) = &args.action {
                let growth = store.starpet_growth()?;
                if !crate::starpet_growth::action_unlocked(growth.xp, action) {
                    return Err(DENIED.into());
                }
            }
            Some(PetSignal {
                source: c.name.clone(),
                status: args.status,
                action: args.action,
                message: args.message.filter(|s| !s.trim().is_empty()),
                expires_at: (now() + duration * 1000).min(c.expires_at),
                owner: c.id.clone(),
            })
        };
        let next = PetSnapshot {
            revision: self
                .pet
                .revision
                .checked_add(1)
                .filter(|n| *n <= 9_007_199_254_740_991)
                .ok_or(DENIED)?,
            signal,
        };
        // Sending fails closed. The desktop callback only targets an existing,
        // pet window; this never starts a window or changes preferences.
        emit(&next)?;
        self.pet = next;
        if action == "pet_signal" {
            if let Some(connection) = self.connections.iter_mut().find(|v| v.id == c.id) {
                connection.last_pet_signal = Some(Instant::now());
            }
            Ok((
                json!({"accepted":true,"revision":self.pet.revision,"expiresAt":self.pet.signal.as_ref().map(|s| s.expires_at)}),
                0,
            ))
        } else {
            Ok((json!({"cleared":true,"revision":self.pet.revision}), 0))
        }
    }
    fn query(
        &self,
        store: &Store,
        c: &Connection,
        action: &str,
        args: Value,
    ) -> Result<(Value, usize)> {
        let args: Arguments = serde_json::from_value(args).map_err(|_| DENIED)?;
        if !TOOLS.contains(&action)
            || args.query.as_ref().is_some_and(|q| q.chars().count() > 512)
            || args.limit.is_some_and(|l| l == 0 || l > c.max_results)
            || (action != "search" && (args.query.is_some() || args.limit.is_some()))
            || (action != "get" && args.id.is_some())
        {
            return Err(DENIED.into());
        }
        if action == "status" {
            let tools: Vec<_> = TOOLS
                .iter()
                .filter(|name| match **name {
                    "status" => true,
                    "pet_signal" | "pet_clear" => c.allow_pet_control,
                    "list_tags" => !c.ids.is_empty() && c.fields.iter().any(|f| f == "tags"),
                    "list_sources" => !c.ids.is_empty() && c.fields.iter().any(|f| f == "sources"),
                    _ => !c.ids.is_empty(),
                })
                .collect();
            return Ok((
                json!({"available":true,"readOnly":!c.allow_pet_control,"clipboardReadOnly":true,"allowPetControl":c.allow_pet_control,"expiresAt":c.expires_at,"maxResults":c.max_results,"fields":c.fields,"tools":tools,"scope":"explicit-records","contentTrust":"untrusted-user-data"}),
                0,
            ));
        }
        if c.ids.is_empty() {
            return Err(DENIED.into());
        }
        let revision = store.privacy_rules().map_err(|_| DENIED)?.revision;
        if action == "get" {
            let id = args.id.ok_or(DENIED)?;
            if !c.ids.contains_key(&id) {
                return Err(DENIED.into());
            }
            let clip = store.clip(id).map_err(|_| DENIED)?;
            if !safe(&clip, revision) || c.ids.get(&id) != Some(&clip.record_identity) {
                return Err(DENIED.into());
            }
            return Ok((render(&clip, c, false), 1));
        }
        // Read only the fixed authorized IDs. No whole-store count or metadata escapes.
        let mut visible = Vec::new();
        for (id, identity) in &c.ids {
            if let Ok(clip) = store.clip(*id) {
                if safe(&clip, revision) && identity == &clip.record_identity {
                    visible.push(clip);
                }
            }
        }
        if action == "search" {
            let query = args.query.unwrap_or_default().to_lowercase();
            let has = |field: &str| c.fields.iter().any(|f| f == field);
            let found: Vec<_> = visible
                .iter()
                .filter(|clip| {
                    query.is_empty()
                        || (has("title") && clip.title.to_lowercase().contains(&query))
                        || (has("content") && clip.content.to_lowercase().contains(&query))
                })
                .collect();
            let rows: Vec<_> = found
                .iter()
                .take(args.limit.unwrap_or(c.max_results))
                .map(|clip| render(clip, c, true))
                .collect();
            let count = rows.len();
            return Ok((
                json!({"records":rows,"total":found.len(),"returned":count,"contentTrust":"untrusted-user-data"}),
                count,
            ));
        }
        let field = if action == "list_tags" {
            "tags"
        } else {
            "sources"
        };
        if !c.fields.iter().any(|f| f == field) {
            return Err(DENIED.into());
        }
        let mut counts = BTreeMap::<String, usize>::new();
        for clip in visible {
            let values = if action == "list_tags" {
                clip.tags
            } else {
                clip.link_hosts
            };
            for value in values {
                *counts.entry(value).or_default() += 1;
            }
        }
        let entries: Vec<_> = counts
            .into_iter()
            .take(c.max_results)
            .map(|(name, count)| json!({"name":name,"count":count}))
            .collect();
        let count = entries.len();
        Ok((json!({"items":entries,"returned":count}), count))
    }
}
fn render(clip: &Clip, c: &Connection, summary: bool) -> Value {
    let mut value = json!({"id":clip.id,"kind":clip.kind,"contentTrust":"untrusted-user-data"});
    for field in &c.fields {
        match field.as_str() {
            "title" => value["title"] = json!(limited(&clip.title, 200)),
            "content" => {
                let limit = if summary { 300 } else { 8000 };
                value["content"] = json!(limited(&clip.content, limit));
                value["truncated"] = json!(clip.content.chars().count() > limit);
            }
            "tags" => value["tags"] = json!(clip.tags),
            "sources" => {
                value["sources"] = json!(clip.link_sources);
                value["hosts"] = json!(clip.link_hosts);
            }
            _ => {}
        }
    }
    value
}
pub const TOOLS: [&str; 7] = [
    "search",
    "get",
    "list_sources",
    "list_tags",
    "status",
    "pet_signal",
    "pet_clear",
];
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub token: String,
    pub action: String,
    #[serde(default = "empty_object")]
    pub arguments: Value,
}
fn empty_object() -> Value {
    json!({})
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Arguments {
    query: Option<String>,
    limit: Option<usize>,
    id: Option<i64>,
}

pub fn read_frame(reader: &mut impl BufRead, max: usize) -> std::io::Result<Option<Vec<u8>>> {
    let mut data = Vec::new();
    loop {
        let buf = reader.fill_buf()?;
        if buf.is_empty() {
            return if data.is_empty() {
                Ok(None)
            } else {
                Err(std::io::ErrorKind::UnexpectedEof.into())
            };
        }
        let count = buf
            .iter()
            .position(|b| *b == b'\n')
            .map_or(buf.len(), |p| p + 1);
        if data.len() + count > max {
            return Err(std::io::ErrorKind::InvalidData.into());
        }
        data.extend_from_slice(&buf[..count]);
        reader.consume(count);
        if data.last() == Some(&b'\n') {
            return Ok(Some(data));
        }
    }
}
pub fn write_frame(writer: &mut impl Write, value: &Value) -> std::io::Result<()> {
    let bytes = serde_json::to_vec(value)?;
    if bytes.len() > MAX_RESPONSE {
        return Err(std::io::ErrorKind::InvalidData.into());
    }
    writer.write_all(&bytes)?;
    writer.write_all(b"\n")?;
    writer.flush()
}
pub fn client(port: u16, token: &str, action: &str, arguments: Value) -> Result<Value> {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let mut stream =
        TcpStream::connect_timeout(&addr, Duration::from_secs(2)).map_err(|_| DENIED)?;
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .map_err(|_| DENIED)?;
    stream
        .set_write_timeout(Some(Duration::from_secs(2)))
        .map_err(|_| DENIED)?;
    let request = serde_json::to_value(Request {
        token: token.into(),
        action: action.into(),
        arguments,
    })
    .map_err(|_| DENIED)?;
    if request.to_string().len() > MAX_REQUEST {
        return Err(DENIED.into());
    }
    write_frame(&mut stream, &request).map_err(|_| DENIED)?;
    let bytes = read_frame(&mut std::io::BufReader::new(stream), MAX_RESPONSE)
        .map_err(|_| DENIED)?
        .ok_or(DENIED)?;
    let result: Value = serde_json::from_slice(&bytes).map_err(|_| DENIED)?;
    if result.get("error").is_some() {
        return Err(DENIED.into());
    }
    result.get("result").cloned().ok_or_else(|| DENIED.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture {
        path: std::path::PathBuf,
        store: Option<Store>,
    }
    impl Fixture {
        fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("starclip-ai-test-{}", random_id().unwrap()));
            std::fs::create_dir_all(&path).unwrap();
            Self {
                store: Some(Store::open(&path.join("fixtures.sqlite3")).unwrap()),
                path,
            }
        }
        fn store(&self) -> &Store {
            self.store.as_ref().unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            self.store.take();
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
    fn request(token: &str, action: &str, arguments: Value) -> Request {
        Request {
            token: token.into(),
            action: action.into(),
            arguments,
        }
    }
    fn connect(
        ai: &mut AiAccess,
        store: &Store,
        ids: Vec<i64>,
        fields: Vec<String>,
    ) -> (String, String) {
        ai.port = Some(54321);
        ai.enable(true).unwrap();
        let created = ai
            .create(
                store,
                CreateConnection {
                    name: "原创测试连接".into(),
                    clip_ids: ids,
                    fields,
                    max_results: 5,
                    expires_hours: 1,
                    allow_pet_control: false,
                },
                Path::new("/fixed/starclip"),
            )
            .unwrap();
        (
            created["config"]["mcp"]["mcpServers"]["starclip"]["env"]["STARCLIP_AI_TOKEN"]
                .as_str()
                .unwrap()
                .into(),
            created["connectionId"].as_str().unwrap().into(),
        )
    }
    fn pet_connection(ai: &mut AiAccess, store: &Store, name: &str) -> (String, String) {
        ai.port = Some(54321);
        ai.enable(true).unwrap();
        let created = ai
            .create(
                store,
                CreateConnection {
                    name: name.into(),
                    clip_ids: vec![],
                    fields: vec![],
                    max_results: 5,
                    expires_hours: 1,
                    allow_pet_control: true,
                },
                Path::new("/fixed/starclip"),
            )
            .unwrap();
        (
            created["config"]["mcp"]["mcpServers"]["starclip"]["env"]["STARCLIP_AI_TOKEN"]
                .as_str()
                .unwrap()
                .into(),
            created["connectionId"].as_str().unwrap().into(),
        )
    }
    fn enable_pet(store: &Store) {
        let mut preferences = store.starpet_preferences().unwrap();
        preferences.enabled = true;
        store.save_starpet_preferences(&preferences).unwrap();
    }
    #[test]
    fn connection_still_requires_all_text_history_reviewed_but_not_images() {
        let fixture = Fixture::new();
        let store = fixture.store();
        store.add("原创未选旧记录", "旧规则记录", None).unwrap();
        let mut rules = store.privacy_rules().unwrap();
        rules.rules.push(crate::privacy::PrivacyRule {
            id: "fixture-scan-check".into(),
            name: "原创扫描规则".into(),
            enabled: true,
            condition: "keyword".into(),
            value: "原创不匹配内容".into(),
            tag: None,
            mark_private: true,
        });
        store.save_privacy_rules(&rules).unwrap();
        let selected = store.add("原创已按新规则扫描", "本次授权", None).unwrap();
        let image = crate::images::prepare_rgba(1, 1, &[30, 60, 90, 255]).unwrap();
        let image = store.add_image(&image, "原创像素", None, false).unwrap();
        assert_eq!(image.scan_version, 0);
        let mut ai = AiAccess::default();
        ai.port = Some(54321);
        ai.enable(true).unwrap();
        let create = || CreateConnection {
            name: "原创审核连接".into(),
            clip_ids: vec![selected.id],
            fields: vec!["title".into()],
            max_results: 5,
            expires_hours: 1,
            allow_pet_control: false,
        };
        assert_eq!(
            ai.create(store, create(), Path::new("/fixed/starclip"))
                .unwrap_err(),
            "请先在规则与隐私中完成历史扫描，再创建连接。"
        );
        assert!(ai.connections.is_empty());
        let scan = store.privacy_rescan_preview().unwrap();
        store.privacy_rescan_apply(&scan.token).unwrap();
        assert!(ai
            .create(store, create(), Path::new("/fixed/starclip"))
            .is_ok());
        assert_eq!(ai.connections[0].record_count, 1);
        assert_eq!(
            ai.connections[0].ids.keys().copied().collect::<Vec<_>>(),
            [selected.id]
        );
    }
    #[test]
    fn pet_permission_is_separate_and_pet_only_does_not_require_history_scan() {
        let fixture = Fixture::new();
        let store = fixture.store();
        let clip = store.add("原创未授权星轨", "星轨", None).unwrap();
        let mut ai = AiAccess::default();
        let (read_token, _) = connect(&mut ai, store, vec![clip.id], vec!["title".into()]);
        let legacy: CreateConnection = serde_json::from_value(json!({"name":"旧协议","clipIds":[clip.id],"fields":["title"],"maxResults":5,"expiresHours":1})).unwrap();
        assert!(!legacy.allow_pet_control);
        let mut rules = store.privacy_rules().unwrap();
        rules.rules.push(crate::privacy::PrivacyRule {
            id: "fixture-pet-only".into(),
            name: "原创规则".into(),
            enabled: true,
            condition: "keyword".into(),
            value: "原创不匹配词".into(),
            tag: None,
            mark_private: true,
        });
        store.save_privacy_rules(&rules).unwrap();
        let (pet_token, _) = pet_connection(&mut ai, store, "原创星伴连接");
        enable_pet(store);
        assert!(ai
            .respond(
                store,
                request(&read_token, "pet_signal", json!({"status":"working"}))
            )
            .get("error")
            .is_some());
        for action in ["search", "get", "list_sources", "list_tags"] {
            assert!(ai
                .respond(store, request(&pet_token, action, json!({"id":clip.id})))
                .get("error")
                .is_some());
        }
        let status = ai.respond(store, request(&pet_token, "status", json!({})));
        assert_eq!(status["result"]["clipboardReadOnly"], true);
        assert_eq!(status["result"]["allowPetControl"], true);
        assert_eq!(status["result"]["readOnly"], false);
        assert_eq!(
            status["result"]["tools"],
            json!(["status", "pet_signal", "pet_clear"])
        );
        assert!(ai
            .respond(
                store,
                request(&pet_token, "pet_signal", json!({"status":"working"}))
            )
            .get("result")
            .is_some());
    }
    #[test]
    fn pet_signals_validate_inputs_throttle_expire_and_never_change_growth() {
        let fixture = Fixture::new();
        let store = fixture.store();
        let mut ai = AiAccess::default();
        let (token, _) = pet_connection(&mut ai, store, "原创 Agent");
        assert!(ai
            .respond(
                store,
                request(&token, "pet_signal", json!({"status":"thinking"}))
            )
            .get("error")
            .is_some());
        enable_pet(store);
        let before = store.starpet_growth().unwrap();
        assert!(crate::starpet_growth::action_unlocked(90, "twirl"));
        assert!(crate::starpet_growth::action_unlocked(
            crate::starpet_growth::MAX_XP,
            "celebrate"
        ));
        let preferences = store.starpet_preferences().unwrap();
        for args in [
            json!({}),
            json!({"durationSeconds":10}),
            json!({"message":"  "}),
            json!({"message":"原".repeat(121)}),
            json!({"message":"不可\n换行"}),
            json!({"status":"unknown"}),
            json!({"status":"working","durationSeconds":0}),
            json!({"status":"working","durationSeconds":61}),
            json!({"status":"working","durationSeconds":1.5}),
            json!({"status":"working","file":"/fixed"}),
            json!({"action":"unknown"}),
            json!({"action":"celebrate"}),
        ] {
            assert!(ai
                .respond(store, request(&token, "pet_signal", args))
                .get("error")
                .is_some());
        }
        let args = json!({"status":"thinking","action":"greet","message":"原创 <b>信号</b>","durationSeconds":60});
        assert!(ai
            .respond_with_pet(store, request(&token, "pet_signal", args.clone()), |_| Err(
                "fixture emit failure".into()
            ))
            .get("error")
            .is_some());
        assert_eq!(ai.pet_snapshot().revision, 0);
        assert!(ai.pet_snapshot().signal.is_none());
        ai.connections[0].expires_at = now() + 5000;
        let mut emitted = None;
        let response = ai.respond_with_pet(
            store,
            request(&token, "pet_signal", args.clone()),
            |snapshot| {
                emitted = Some(snapshot.clone());
                Ok(())
            },
        );
        assert_eq!(response["result"]["accepted"], true);
        let emitted = emitted.unwrap();
        assert_eq!(
            emitted.signal.as_ref().unwrap().message.as_deref(),
            Some("原创 <b>信号</b>")
        );
        assert_eq!(
            emitted.signal.as_ref().unwrap().expires_at,
            ai.connections[0].expires_at
        );
        assert!(ai
            .respond(store, request(&token, "pet_signal", args))
            .get("error")
            .is_some());
        assert_eq!(ai.pet_snapshot().revision, emitted.revision);
        assert_eq!(store.starpet_growth().unwrap(), before);
        assert_eq!(store.starpet_preferences().unwrap(), preferences);
        assert!(!ai.state().to_string().contains("原创 <b>信号</b>"));
        assert!(!ai.state().to_string().contains(&token));
        ai.pet.signal.as_mut().unwrap().expires_at = 0;
        assert!(ai.pet_snapshot().signal.is_none());
        ai.pet.signal.as_mut().unwrap().expires_at = now() + 5000;
        ai.connections[0].expires_at = 0;
        assert!(ai.pet_snapshot().signal.is_none());
        assert!(ai
            .respond(
                store,
                request(&token, "pet_signal", json!({"status":"idle"}))
            )
            .get("error")
            .is_some());
    }
    #[test]
    fn pet_latest_signal_ownership_revoke_disable_and_local_clear() {
        let fixture = Fixture::new();
        let store = fixture.store();
        enable_pet(store);
        let mut ai = AiAccess::default();
        let (a, aid) = pet_connection(&mut ai, store, "原创 A");
        let (b, bid) = pet_connection(&mut ai, store, "原创 B");
        ai.respond(
            store,
            request(&a, "pet_signal", json!({"status":"working"})),
        );
        ai.respond(
            store,
            request(&b, "pet_signal", json!({"status":"waiting"})),
        );
        assert_eq!(ai.pet_snapshot().signal.as_ref().unwrap().source, "原创 B");
        let revision = ai.pet_snapshot().revision;
        assert_eq!(
            ai.respond(store, request(&a, "pet_clear", json!({})))["result"]["cleared"],
            false
        );
        assert_eq!(ai.pet_snapshot().revision, revision);
        ai.revoke(&aid);
        assert_eq!(ai.pet_snapshot().revision, revision);
        assert!(ai
            .respond(store, request(&b, "pet_clear", json!({"all":true})))
            .get("error")
            .is_some());
        assert_eq!(
            ai.respond(store, request(&b, "pet_clear", json!({})))["result"]["cleared"],
            true
        );
        assert!(ai.pet_snapshot().signal.is_none());
        ai.connections[0].last_pet_signal = None;
        ai.respond(
            store,
            request(&b, "pet_signal", json!({"message":"原创完成"})),
        );
        ai.revoke(&bid);
        assert!(ai.pet_snapshot().signal.is_none());
        let (c, _) = pet_connection(&mut ai, store, "原创 C");
        ai.respond(
            store,
            request(&c, "pet_signal", json!({"status":"success"})),
        );
        ai.clear_pet();
        assert!(ai.pet_snapshot().signal.is_none());
        ai.connections[0].last_pet_signal = None;
        ai.respond(store, request(&c, "pet_signal", json!({"status":"error"})));
        ai.enable(false).unwrap();
        assert!(ai.pet_snapshot().signal.is_none());
        ai.enable(true).unwrap();
        assert!(ai
            .respond(store, request(&c, "pet_signal", json!({"status":"idle"})))
            .get("error")
            .is_some());
    }
    #[test]
    fn mixed_library_filters_every_surface_and_never_exposes_tokens_in_state() {
        let fixture = Fixture::new();
        let store = fixture.store();
        let public = store
            .add("原创天文笔记 https://linux.do/t/fixed", "星轨", None)
            .unwrap();
        store.set_clip_tags(public.id, &["星空".into()]).unwrap();
        let private = store
            .add("原创私密 https://private.example/fixture", "私密标题", None)
            .unwrap();
        store
            .set_clip_tags(private.id, &["不可泄漏标签".into()])
            .unwrap();
        store.set_clip_privacy(private.id, true, false).unwrap();
        let other = store
            .add("未授权的原创 https://unshared.example/", "其他标题", None)
            .unwrap();
        let image = crate::images::prepare_rgba(1, 1, &[12, 34, 56, 255]).unwrap();
        let image = store.add_image(&image, "原创像素", None, false).unwrap();
        let file = store
            .capture_files(
                &["/original-fixture/private-file.txt".into()],
                &crate::privacy::CaptureContext::default(),
            )
            .unwrap();
        let mut ai = AiAccess::default();
        assert!(!ai.enabled);
        let (token, id) = connect(
            &mut ai,
            store,
            vec![public.id],
            FIELDS.iter().map(|s| s.to_string()).collect(),
        );
        assert!(!ai.state().to_string().contains(&token));
        for forbidden in [private.id, other.id, image.id, file.id, 99999] {
            assert_eq!(
                ai.respond(store, request(&token, "get", json!({"id":forbidden}))),
                json!({"error":DENIED})
            );
        }
        let results = ai.respond(store, request(&token, "search", json!({})));
        assert_eq!(results["result"]["total"], 1);
        assert_eq!(results["result"]["records"][0]["id"], public.id);
        assert_eq!(
            ai.respond(store, request(&token, "list_sources", json!({})))["result"]["items"],
            json!([{"name":"linux.do","count":1}])
        );
        assert_eq!(
            ai.respond(store, request(&token, "list_tags", json!({})))["result"]["items"],
            json!([{"name":"星空","count":1}])
        );
        for args in [
            json!({"cursor":"1"}),
            json!({"human":true}),
            json!({"limit":100}),
            json!({"query":"q".repeat(513)}),
        ] {
            assert!(ai
                .respond(store, request(&token, "search", args))
                .get("error")
                .is_some());
        }
        assert!(ai
            .respond(
                store,
                request(&token, "get_clip_image", json!({"id":image.id}))
            )
            .get("error")
            .is_some());
        let unsafe_request = CreateConnection {
            name: "图片".into(),
            clip_ids: vec![image.id],
            fields: vec!["content".into()],
            max_results: 1,
            expires_hours: 1,
            allow_pet_control: false,
        };
        assert!(ai
            .create(store, unsafe_request, Path::new("/fixed/starclip"))
            .is_err());
        let file_request = CreateConnection {
            name: "文件".into(),
            clip_ids: vec![file.id],
            fields: vec!["content".into()],
            max_results: 1,
            expires_hours: 1,
            allow_pet_control: false,
        };
        assert!(ai
            .create(store, file_request, Path::new("/fixed/starclip"))
            .is_err());
        ai.revoke(&id);
        assert!(ai
            .respond(store, request(&token, "status", json!({})))
            .get("error")
            .is_some());
        assert!(!ai.state().to_string().contains("unshared.example"));
        assert!(!ai.state().to_string().contains("私密标题"));
    }
    #[test]
    fn deleted_ids_even_with_identical_content_never_grant_new_records() {
        let fixture = Fixture::new();
        let store = fixture.store();
        let original = store.add("原创固定重复片段", "原创标题", None).unwrap();
        let mut ai = AiAccess::default();
        let (token, _) = connect(&mut ai, store, vec![original.id], vec!["content".into()]);
        assert!(ai
            .respond(store, request(&token, "get", json!({"id":original.id})))
            .get("result")
            .is_some());
        store.delete(original.id).unwrap();
        let recreated = store.add("原创固定重复片段", "原创标题", None).unwrap();
        assert_eq!(
            original.id, recreated.id,
            "SQLite reuses its largest deleted row ID"
        );
        assert_ne!(original.record_identity, recreated.record_identity);
        assert!(ai
            .respond(store, request(&token, "get", json!({"id":recreated.id})))
            .get("error")
            .is_some());
        assert_eq!(
            ai.respond(store, request(&token, "search", json!({})))["result"]["total"],
            0
        );
    }

    #[test]
    fn privacy_revision_revocation_expiry_and_field_permissions_are_checked_each_time() {
        let fixture = Fixture::new();
        let store = fixture.store();
        let clip = store
            .add("原创隐藏内容 https://linux.do/fixture", "可见标题", None)
            .unwrap();
        let mut ai = AiAccess::default();
        let (token, _) = connect(&mut ai, store, vec![clip.id], vec!["title".into()]);
        assert!(
            ai.respond(store, request(&token, "get", json!({"id":clip.id})))["result"]
                .get("content")
                .is_none()
        );
        assert_eq!(
            ai.respond(
                store,
                request(&token, "search", json!({"query":"隐藏内容"}))
            )["result"]["total"],
            0
        );
        assert!(ai
            .respond(store, request(&token, "list_sources", json!({})))
            .get("error")
            .is_some());
        let mut rules = store.privacy_rules().unwrap();
        rules.rules.push(crate::privacy::PrivacyRule {
            id: "fixture-added".into(),
            name: "原创无命中规则".into(),
            enabled: true,
            condition: "keyword".into(),
            value: "不存在的原创关键字".into(),
            tag: None,
            mark_private: true,
        });
        store.save_privacy_rules(&rules).unwrap();
        assert!(ai
            .respond(store, request(&token, "get", json!({"id":clip.id})))
            .get("error")
            .is_some());
        let scan = store.privacy_rescan_preview().unwrap();
        store.privacy_rescan_apply(&scan.token).unwrap();
        assert!(ai
            .respond(store, request(&token, "get", json!({"id":clip.id})))
            .get("result")
            .is_some());
        store.set_clip_privacy(clip.id, true, false).unwrap();
        assert_eq!(
            ai.respond(store, request(&token, "search", json!({})))["result"]["total"],
            0
        );
        ai.connections[0].expires_at = 0;
        assert!(ai
            .respond(store, request(&token, "status", json!({})))
            .get("error")
            .is_some());
        ai.enable(false).unwrap();
        assert!(ai.connections.is_empty());
        ai.enable(true).unwrap();
        assert!(ai
            .respond(store, request(&token, "status", json!({})))
            .get("error")
            .is_some());
    }
}
