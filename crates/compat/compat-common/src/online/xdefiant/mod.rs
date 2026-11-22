pub mod params;
pub mod store;
pub mod voice;

use super::util::{b64, hex, iso_in, iso_now, now_millis, random_uuid, sha1};
use super::ws::{Outlet, WsReply, WsSession};
use super::{Request, Response, TitleService};
use serde_json::{json, Value};
use std::sync::Mutex;
use store::State;

pub const DEFAULT_SPACE_ID: &str = "1f76510f-79cd-4944-86dd-b58dd2a18212";
pub const XBOX_ID: &str = "2533274790395905";

pub const EVENT_TYPES: &[&str] = &[
    "game.start", "game.ad.start", "game.ad.stop", "game.consentPopUp", "game.consentPopUpDisplay", "game.suspendedMode.start", "game.suspendedMode.stop",
    "hardwareSpecs.xboxSeries", "player.start", "player.achievement", "player.chatService", "player.contentDownload", "player.news.action",
    "player.news.impression", "player.primaryStore.xboxSeries", "player.progression", "player.stats.playtime",
];

pub fn notifications_requested() -> bool {
    std::env::var("TOMBOX_XDEFIANT_NOTIFICATIONS").ok().or_else(|| crate::cfg("xdefiant_notifications")).map_or(true, |v| !matches!(v.trim().to_ascii_lowercase().as_str(), "0" | "false" | "off" | "no"))
}

pub fn uuid_for(seed: &str) -> String {
    let h = hex(&sha1(format!("tombox-ubi:{seed}").as_bytes()));
    format!("{}-{}-5{}-a{}-{}", &h[0..8], &h[8..12], &h[13..16], &h[17..20], &h[20..32])
}

pub fn is_uuid(s: &str) -> bool {
    s.len() == 36 && s.bytes().enumerate().all(|(i, b)| if matches!(i, 8 | 13 | 18 | 23) { b == b'-' } else { b.is_ascii_hexdigit() })
}

fn clean_name(v: String) -> Option<String> {
    let v = v.trim().to_string();
    (!v.is_empty() && v.len() <= 32 && v.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-' || b == b'.')).then_some(v)
}

pub fn default_name() -> String {
    std::env::var("TOMBOX_ONLINE_NAME").ok().or_else(|| crate::cfg("online_display_name")).and_then(clean_name).unwrap_or_else(|| "XboxEmulator".into())
}

pub struct XDefiant {
    pub name: String,
    pub profile_id: String,
    pub space_id: String,
    pub notifications: bool,
    pub state: Mutex<State>,
}

pub fn ubi_error(status: u16, code: u32, message: &str) -> Response {
    Response::json_status(status, &json!({
        "errorCode": code,
        "message": message,
        "httpCode": status,
        "errorContext": "Profiles Services",
        "moreInfo": "",
        "transactionTime": iso_now(),
        "transactionId": random_uuid(),
        "environment": "Prod",
    }))
}

fn created(v: &Value) -> Response {
    Response::json_status(201, v)
}

fn list_of(v: &Value) -> Vec<Value> {
    match v {
        Value::Array(a) => a.clone(),
        Value::Null => Vec::new(),
        o => vec![o.clone()],
    }
}

fn csv(req: &Request, keys: &[&str]) -> Vec<String> {
    let mut out = Vec::new();
    for k in keys {
        for v in req.query_all(k) {
            out.extend(v.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()));
        }
    }
    out
}

impl XDefiant {
    pub fn new() -> XDefiant {
        let space = crate::cfg("xdefiant_space_id").filter(|s| is_uuid(s)).unwrap_or_else(|| DEFAULT_SPACE_ID.into());
        let mut x = XDefiant::with(&default_name(), &space);
        x.notifications = notifications_requested();
        x
    }

    pub fn with(name: &str, space_id: &str) -> XDefiant {
        let profile_id = uuid_for(name);
        XDefiant { name: name.to_string(), state: Mutex::new(State::new(&profile_id)), profile_id, space_id: space_id.to_string(), notifications: true }
    }

    fn st(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn session_ticket(&self) -> Option<String> {
        self.st().session.as_ref().and_then(|s| s["ticket"].as_str().map(String::from))
    }

    fn platform(req: &Request) -> String {
        req.header("ubi-requestedplatformtype").map(|s| s.to_ascii_lowercase()).filter(|s| !s.is_empty()).unwrap_or_else(|| "xbl".into())
    }

    fn me(&self, id: &str) -> bool {
        id == "me" || id.eq_ignore_ascii_case(&self.profile_id) || id == XBOX_ID
    }

    fn space(&self, req: &Request) -> String {
        req.query("spaceId").filter(|s| is_uuid(s)).map(|s| s.to_string()).unwrap_or_else(|| self.space_id.clone())
    }

    pub fn profile(&self, platform: &str) -> Value {
        json!({
            "profileId": self.profile_id,
            "userId": self.profile_id,
            "platformType": platform,
            "idOnPlatform": if platform == "uplay" { self.profile_id.clone() } else { XBOX_ID.to_string() },
            "nameOnPlatform": self.name,
        })
    }

    fn known_profile(&self, id: &str, platform: &str) -> Option<Value> {
        if self.me(id) {
            return Some(self.profile(platform));
        }
        self.st().profiles.get(&id.to_ascii_lowercase()).cloned()
    }

    fn new_session(&self, req: &Request) -> Value {
        let ticket = b64(format!("tombox-local-ticket:{}:{}:{}", self.profile_id, now_millis(), random_uuid()).as_bytes());
        let space = req.header("ubi-spaceid").filter(|s| is_uuid(s)).map(|s| s.to_string()).unwrap_or_else(|| self.space_id.clone());
        let s = json!({
            "platformType": Self::platform(req),
            "ticket": ticket,
            "twoFactorAuthenticationTicket": Value::Null,
            "profileId": self.profile_id,
            "userId": self.profile_id,
            "nameOnPlatform": self.name,
            "environment": "Prod",
            "expiration": iso_in(3 * 3600),
            "spaceId": space,
            "clientIp": "127.0.0.1",
            "clientIpCountry": "US",
            "serverTime": iso_now(),
            "sessionId": random_uuid(),
            "sessionKey": b64(&sha1(format!("{}:{}", self.profile_id, now_millis()).as_bytes())),
            "rememberMeTicket": Value::Null,
        });
        self.st().session = Some(s.clone());
        s
    }

    fn extend_session(&self, req: &Request) -> Value {
        let cur = self.st().session.clone();
        match cur {
            Some(mut s) => {
                s["expiration"] = json!(iso_in(3 * 3600));
                s["serverTime"] = json!(iso_now());
                self.st().session = Some(s.clone());
                s
            }
            None => self.new_session(req),
        }
    }

    fn profiles_lookup(&self, req: &Request) -> Value {
        let platform = req.query("platformType").map(|s| s.to_ascii_lowercase()).unwrap_or_else(|| Self::platform(req));
        let mut ids = csv(req, &["userId", "userIds", "profileId", "profileIds", "idOnPlatform", "idsOnPlatform"]);
        if req.method == "POST" {
            let b = req.json();
            for k in ["profileIds", "userIds", "idsOnPlatform"] {
                ids.extend(list_of(&b[k]).iter().filter_map(|v| v.as_str().map(|s| s.to_string())));
            }
        }
        let names = csv(req, &["nameOnPlatform", "namesOnPlatform"]);
        let mut out = Vec::new();
        for n in &names {
            if n.eq_ignore_ascii_case(&self.name) {
                out.push(self.profile(&platform));
            }
        }
        for id in ids {
            if let Some(p) = self.known_profile(&id, &platform) {
                if !out.contains(&p) {
                    out.push(p);
                }
            }
        }
        if out.is_empty() && names.is_empty() && req.query_all("profileIds").is_empty() && req.query_all("userIds").is_empty() && req.query_all("idOnPlatform").is_empty() {
            out.push(self.profile(&platform));
        }
        json!({ "profiles": out })
    }

    fn user(&self) -> Value {
        json!({
            "userId": self.profile_id,
            "username": self.name,
            "nameOnPlatform": self.name,
            "country": "US",
            "preferredLanguage": "en",
            "accountType": "Ubisoft",
            "status": { "autoGeneratedUsername": false, "dateOfBirthApproved": true, "invalidEmail": false, "locked": false, "pendingDeactivation": false, "minor": false, "generalStatus": "activated" },
            "dateCreated": "2023-05-23T00:00:00.000Z",
        })
    }

    fn application(&self, id: &str) -> Value {
        json!({
            "applicationId": id,
            "name": "XDefiant",
            "platform": "XBOXSERIES",
            "spaceId": self.space_id,
            "overrideConnectionInfo": Value::Null,
        })
    }

    fn event_config(&self) -> Value {
        json!({
            "publicConfig": { "sendPeriodSeconds": 60, "playtimeEventPeriodSeconds": 300, "hmacEnabled": false },
            "types": EVENT_TYPES,
        })
    }

    fn event_definitions(&self, req: &Request) -> Value {
        let wanted = csv(req, &["names", "name", "eventNames"]);
        let defs: Vec<Value> = EVENT_TYPES.iter().filter(|t| wanted.is_empty() || wanted.iter().any(|w| w.eq_ignore_ascii_case(t))).map(|t| json!({
            "name": t,
            "version": 1,
            "enabled": true,
            "sendPeriodSeconds": 60,
            "schema": { "type": "object", "additionalProperties": true },
        })).collect();
        json!({ "spaceId": self.space(req), "eventsDefinitions": defs })
    }

    fn notify(&self, target: &str, body: &Value, space: &str) -> Value {
        let ty = body["notificationType"].as_str().or_else(|| body["type"].as_str()).unwrap_or("CUSTOM").to_string();
        let env = json!({
            "notificationType": ty,
            "content": if body["content"].is_null() { body.clone() } else { body["content"].clone() },
            "datePosted": iso_now(),
            "spaceId": body["spaceId"].as_str().unwrap_or(space),
            "sourceProfileId": self.profile_id,
            "sourceApplicationId": body["sourceApplicationId"].clone(),
        });
        let target = if self.me(target) { self.profile_id.clone() } else { target.to_ascii_lowercase() };
        let delivered = super::ws::publish(&format!("ubi:{target}"), &env.to_string());
        let rec = json!({ "targetProfileId": target, "notification": env, "delivered": delivered });
        self.st().push("notifications", rec.clone());
        rec
    }

    fn route_v(&self, req: &Request, m: &str, seg: &[&str]) -> Option<Response> {
        let platform = XDefiant::platform(req);
        let body = || req.json();
        let r = match (m, seg) {
            ("GET", ["tombox", "request-info"]) => Response::json(&json!({
                "method": req.method,
                "path": req.path,
                "forwardedHost": req.header("x-forwarded-host"),
                "appId": req.header("ubi-appid"),
                "authorization": req.header("authorization").map(|a| a.split_whitespace().next().unwrap_or("").to_string()),
            })),
            ("GET", ["tombox", "state"]) => Response::json(&self.st().dump()),
            ("GET", ["tombox", "telemetry"]) => {
                let s = self.st();
                Response::json(&json!({ "events": s.list("events"), "remoteLogs": s.list("remoteLogs"), "notifications": s.list("notifications") }))
            }
            ("GET", ["tombox", "upload", id]) => match self.st().files.get(*id) {
                Some(b) => Response::bytes(200, "application/octet-stream", b.clone()),
                None => ubi_error(404, 1000, "file not found"),
            },
            ("PUT" | "POST", ["tombox", "upload", id]) => {
                let mut s = self.st();
                if s.files.len() >= 512 {
                    return Some(ubi_error(507, 1001, "upload store full"));
                }
                s.files.insert(id.to_string(), req.body.clone());
                Response::json(&json!({ "fileId": id, "size": req.body.len() }))
            }

            ("POST", ["profiles", "sessions"]) => Response::json(&self.new_session(req)),
            ("PUT", ["profiles", "sessions"]) => Response::json(&self.extend_session(req)),
            ("GET", ["profiles", "sessions"]) => match self.st().session.clone() {
                Some(s) => Response::json(&s),
                None => ubi_error(401, 1, "Invalid session"),
            },
            ("DELETE", ["profiles", "sessions"]) => {
                self.st().session = None;
                Response::empty(200)
            }

            ("GET", ["applications", app, "configuration"]) => Response::json(&params::configuration(self, app, req.query("parameterGroups"))),
            ("GET", ["applications", _app, "parameters"]) => Response::json(&params::application_parameters(self, req.query("parameterGroups"))),
            ("GET", ["spaces", space, "parameters"]) => Response::json(&params::space_parameters(self, space, req.query("parameterGroups"))),
            ("GET", ["applications"]) => {
                let ids = csv(req, &["applicationIds", "applicationId"]);
                let ids = if ids.is_empty() { req.header("ubi-appid").map(|s| vec![s.to_string()]).unwrap_or_default() } else { ids };
                Response::json(&json!({ "applications": ids.iter().map(|i| self.application(i)).collect::<Vec<_>>() }))
            }
            ("GET", ["applications", app]) => Response::json(&self.application(app)),

            ("GET", ["profiles"]) | ("GET" | "POST", ["profiles", "external"]) => Response::json(&self.profiles_lookup(req)),
            ("GET", ["profiles", id]) if is_uuid(id) || self.me(id) => match self.known_profile(id, &platform) {
                Some(p) => Response::json(&p),
                None => ubi_error(404, 1002, "Profile not found"),
            },

            ("GET", ["users", "me"]) => Response::json(&self.user()),
            ("GET", ["users", id]) if self.me(id) => Response::json(&self.user()),
            ("GET", ["users"]) | ("GET" | "POST", ["users", "batch"]) => Response::json(&json!({ "users": [self.user()] })),
            ("PUT", ["users", "me", "onlinestatuses"]) | ("PUT", ["users", "me", "onlinestatuses", "manualstatus"]) => {
                let mut v = body();
                v["userId"] = json!(self.profile_id);
                v["lastModified"] = json!(iso_now());
                v["manuallySet"] = json!(seg.len() == 4);
                self.st().set_doc("onlineStatus", &self.profile_id, v.clone());
                Response::json(&v)
            }
            ("GET", ["users", "onlinestatuses"]) | ("GET", ["users", "me", "onlinestatuses"]) => {
                let s = self.st();
                let mine = s.doc("onlineStatus", &self.profile_id).unwrap_or_else(|| json!({ "userId": self.profile_id, "onlineStatus": "online", "manuallySet": false }));
                Response::json(&json!({ "onlineStatuses": [mine] }))
            }

            ("GET", ["policies"]) => Response::json(&json!({ "policies": self.st().list("policies") })),

            ("PUT" | "POST", ["profiles", id, "populations", "data"]) if self.me(id) => {
                let b = body();
                let space = b["spaceId"].as_str().unwrap_or(&self.space_id).to_string();
                self.st().set_doc("populationData", &space, b);
                Response::json(&json!({ "populations": self.st().list("populations") }))
            }
            ("GET", ["profiles", id, "populations"]) if self.me(id) => Response::json(&json!({ "populations": self.st().list("populations") })),

            ("POST", ["profiles", id, "events"]) if self.me(id) => {
                let b = body();
                let evs = if b["events"].is_array() { list_of(&b["events"]) } else { list_of(&b) };
                let n = evs.len();
                let mut s = self.st();
                for e in evs {
                    s.push("events", json!({ "receivedAt": iso_now(), "event": e }));
                }
                drop(s);
                crate::record_raw("online", "xdefiant-telemetry", &format!("stored {n} events"));
                Response::empty(204)
            }
            ("GET", ["spaces", _, "configs", "events"]) => Response::json(&self.event_config()),
            ("GET", ["spaces", _, "configs", "events", "definitions"]) => Response::json(&self.event_definitions(req)),
            ("GET", ["spaces", sid, "configs", "voicechat", "vivox"]) => Response::json(&voice::config(sid)),
            ("GET" | "POST", ["profiles", pid, "voicechattoken", "vivox"]) => {
                if !self.me(pid) {
                    return Some(ubi_error(403, 1010, "Voice chat tokens are issued only for the signed-in profile"));
                }
                match voice::token(&self.profile_id, &self.space(req), req) {
                    Ok(t) => {
                        self.st().push("voiceTokens", json!({ "issuedAt": iso_now(), "action": t["action"].clone(), "channel": t["channel"].clone() }));
                        Response::json(&t)
                    }
                    Err(msg) => ubi_error(400, 1011, &msg),
                }
            }
            ("GET", ["spaces", _, "configs", "primarystore"]) => Response::json(&json!({ "mappings": self.st().list("primarystoreMappings") })),
            ("POST", ["profiles", id, "remotelog"]) | ("POST", ["profiles", id, "remotelogs"]) if self.me(id) => {
                let b = if req.json().is_null() { json!(req.body_str()) } else { req.json() };
                self.st().push("remoteLogs", json!({ "receivedAt": iso_now(), "log": b }));
                Response::empty(204)
            }

            ("GET", ["profiles", pid, "entities"]) => {
                let owner = if self.me(pid) { self.profile_id.clone() } else { pid.to_ascii_lowercase() };
                Response::json(&json!({ "entities": self.st().entities(Some(&owner), None, req) }))
            }
            ("GET", ["profiles", "entities"]) => {
                let owners = csv(req, &["profileIds", "profileId"]);
                let s = self.st();
                let mut out = Vec::new();
                for o in owners {
                    let o = if self.me(&o) { self.profile_id.clone() } else { o.to_ascii_lowercase() };
                    out.extend(s.entities(Some(&o), None, req));
                }
                Response::json(&json!({ "entities": out }))
            }
            ("POST", ["profiles", pid, "entities"]) if self.me(pid) => created(&self.st().create_entity(Some(&self.profile_id), &self.space(req), body())),
            ("GET", ["spaces", sid, "entities"]) => Response::json(&json!({ "entities": self.st().entities(None, Some(sid), req) })),
            ("POST", ["spaces", sid, "entities"]) => created(&self.st().create_entity(None, sid, body())),
            ("GET", ["profiles", _, "entities", eid]) | ("GET", ["spaces", _, "entities", eid]) => match self.st().entity(eid) {
                Some(e) => Response::json(&e),
                None => ubi_error(404, 1003, "Entity not found"),
            },
            ("PUT" | "PATCH", ["profiles", _, "entities", eid]) | ("PUT" | "PATCH", ["spaces", _, "entities", eid]) => match self.st().update_entity(eid, body()) {
                Some(e) => Response::json(&e),
                None => ubi_error(404, 1003, "Entity not found"),
            },
            ("DELETE", ["profiles", _, "entities", eid]) | ("DELETE", ["spaces", _, "entities", eid]) => {
                if self.st().delete_entity(eid) { Response::empty(200) } else { ubi_error(404, 1003, "Entity not found") }
            }

            ("GET", ["profiles", pid, "stats"]) => {
                let owner = if self.me(pid) { self.profile_id.clone() } else { pid.to_ascii_lowercase() };
                Response::json(&json!({ "profileId": owner, "stats": self.st().stats(&owner, &csv(req, &["statNames", "statName"])) }))
            }
            ("PUT" | "POST", ["profiles", pid, "stats"]) if self.me(pid) => {
                let b = body();
                self.st().put_stats(&self.profile_id, &b["stats"]);
                Response::json(&json!({ "profileId": self.profile_id, "stats": self.st().stats(&self.profile_id, &[]) }))
            }
            ("GET", ["profiles", "stats"]) => {
                let s = self.st();
                let out: Vec<Value> = csv(req, &["profileIds"]).iter().map(|p| {
                    let p = if self.me(p) { self.profile_id.clone() } else { p.to_ascii_lowercase() };
                    json!({ "profileId": p, "stats": s.stats(&p, &csv(req, &["statNames"])) })
                }).collect();
                Response::json(&json!({ "profiles": out }))
            }

            ("GET", ["profiles", pid, "friends"]) if self.me(pid) => Response::json(&json!({ "friends": self.st().list("friends") })),
            ("POST" | "PUT", ["profiles", pid, "friends"]) if self.me(pid) => {
                let b = body();
                let mut added = Vec::new();
                for f in list_of(if b["friends"].is_null() { &b } else { &b["friends"] }) {
                    let id = f["pid"].as_str().or_else(|| f["profileId"].as_str()).unwrap_or("").to_ascii_lowercase();
                    if !is_uuid(&id) || self.me(&id) {
                        continue;
                    }
                    let rec = json!({ "pid": id, "profileId": id, "state": "PendingSentInvitation", "nameOnPlatform": f["nameOnPlatform"].clone(), "createdAt": iso_now() });
                    self.st().upsert("friends", "pid", rec.clone());
                    self.notify(&id, &json!({ "notificationType": "FRIENDS_RELATIONSHIP_UPDATE", "content": { "pid": self.profile_id, "state": "PendingReceivedInvitation" } }), &self.space_id);
                    added.push(rec);
                }
                Response::json(&json!({ "friends": added }))
            }
            ("DELETE", ["profiles", pid, "friends", fid]) if self.me(pid) => {
                if self.st().remove("friends", "pid", &fid.to_ascii_lowercase()) { Response::empty(200) } else { ubi_error(404, 1004, "Friend not found") }
            }
            ("GET", ["profiles", "me", "blocklist", ..]) => Response::json(&json!({ "blocklist": self.st().list("blocklist") })),
            ("POST" | "PUT", ["profiles", "me", "blocklist"]) => {
                let b = body();
                let mut out = Vec::new();
                for id in list_of(if b["profileIds"].is_null() { &b["profileId"] } else { &b["profileIds"] }) {
                    if let Some(id) = id.as_str() {
                        let rec = json!({ "profileId": id.to_ascii_lowercase(), "blockedAt": iso_now() });
                        self.st().upsert("blocklist", "profileId", rec.clone());
                        out.push(rec);
                    }
                }
                Response::json(&json!({ "blocklist": out }))
            }
            ("DELETE", ["profiles", "me", "blocklist", id]) => {
                if self.st().remove("blocklist", "profileId", &id.to_ascii_lowercase()) { Response::empty(200) } else { ubi_error(404, 1005, "Not blocked") }
            }

            ("GET", ["profiles", pid, "inventory"]) | ("GET", ["profiles", pid, "inventory", "primarystore"]) | ("GET", ["profiles", pid, "inventory", "instances"]) => {
                if !self.me(pid) {
                    return Some(ubi_error(403, 1006, "Inventory of other profiles is private"));
                }
                Response::json(&json!({ "items": self.st().list(if seg.len() == 4 && seg[3] == "instances" { "instances" } else { "inventory" }) }))
            }
            ("GET", ["profiles", pid, "inventory", "transactions"]) if self.me(pid) => Response::json(&json!({ "transactions": self.st().list("transactions") })),
            ("POST", ["profiles", pid, "inventory", "transactions"]) | ("POST", ["profiles", pid, "inventory", "instances", "transactions"]) if self.me(pid) => {
                let t = self.st().apply_transaction(body());
                self.notify(&self.profile_id.clone(), &json!({ "notificationType": "US_CLIENT_SECONDARY_STORE_UPDATE", "content": { "transactionId": t["transactionId"].clone() } }), &self.space_id);
                created(&t)
            }
            ("GET", ["profiles", pid, "inventory", "expireddetails"]) if self.me(pid) => Response::json(&json!({ "items": self.st().expired_items() })),

            ("GET", ["spaces", sid, "battlepasses", "seasons"]) | ("GET", ["spaces", sid, "battlepasses"]) => Response::json(&json!({ "seasons": self.st().seasons(sid) })),
            ("GET", ["spaces", sid, "battlepasses", "seasons", season]) => match self.st().seasons(sid).into_iter().find(|s| s["seasonId"] == json!(season)) {
                Some(s) => Response::json(&s),
                None => ubi_error(404, 1007, "Season not found"),
            },
            ("GET", ["profiles", pid, "battlepasses", "seasons", ..]) if self.me(pid) => {
                let space = self.space(req);
                Response::json(&json!({ "seasons": self.st().season_progress(&space) }))
            }

            ("GET", ["profiles", pid, "playerpreferences", "standard"]) => {
                let owner = if self.me(pid) { self.profile_id.clone() } else { pid.to_ascii_lowercase() };
                Response::json(&self.st().preferences(&owner, &self.space(req)))
            }
            ("PUT" | "PATCH" | "POST", ["profiles", pid, "playerpreferences", "standard"]) if self.me(pid) => {
                let space = self.space(req);
                Response::json(&self.st().put_preferences(&self.profile_id, &space, body()))
            }
            ("GET", ["spaces", sid, "playerpreferences", "standard"]) => Response::json(&self.st().preferences("space", sid)),

            ("GET", ["profiles", pid, "privileges"]) => {
                let owner = if self.me(pid) { self.profile_id.clone() } else { pid.to_ascii_lowercase() };
                Response::json(&json!({ "profileId": owner, "privileges": store::PRIVILEGES.iter().map(|p| json!({ "name": p, "granted": true })).collect::<Vec<_>>() }))
            }

            ("GET", ["profiles", "richpresences"]) | ("GET", ["spaces", _, "richpresences"]) => {
                let ids = csv(req, &["profileIds", "profileId"]);
                let s = self.st();
                let out: Vec<Value> = if ids.is_empty() { s.list("richPresence") } else { ids.iter().filter_map(|i| s.doc("richPresenceBy", &if self.me(i) { self.profile_id.clone() } else { i.to_ascii_lowercase() })).collect() };
                Response::json(&json!({ "richPresences": out }))
            }
            ("PUT" | "POST", ["profiles", pid, "richpresences"]) | ("PUT" | "POST", ["spaces", _, "richpresences", pid]) if self.me(pid) => {
                let mut v = body();
                v["profileId"] = json!(self.profile_id);
                v["lastModified"] = json!(iso_now());
                let mut s = self.st();
                s.set_doc("richPresenceBy", &self.profile_id, v.clone());
                s.upsert("richPresence", "profileId", v.clone());
                Response::json(&v)
            }

            ("GET", ["profiles", "connections"]) => {
                let ids = csv(req, &["profileIds", "profileId"]);
                let ids: Vec<String> = if ids.is_empty() { vec![self.profile_id.clone()] } else { ids.iter().map(|i| if self.me(i) { self.profile_id.clone() } else { i.to_ascii_lowercase() }).collect() };
                Response::json(&json!({ "connections": store::connections_for(&ids) }))
            }
            ("GET", ["profiles", pid, "connections"]) => {
                let owner = if self.me(pid) { self.profile_id.clone() } else { pid.to_ascii_lowercase() };
                Response::json(&json!({ "connections": store::connections_for(&[owner]) }))
            }
            ("PUT" | "POST" | "PATCH", ["profiles", pid, "connections"]) | ("PUT" | "POST" | "PATCH", ["profiles", pid, "connections", _]) if self.me(pid) => {
                let cid = seg.get(3).map(|s| s.to_string());
                match store::update_connection(&self.profile_id, cid.as_deref(), body()) {
                    Some(c) => Response::json(&c),
                    None => ubi_error(404, 1008, "No websocket connection is open for this profile"),
                }
            }
            ("PUT" | "POST" | "PATCH", ["profiles", "connections", cid]) => {
                let owner = store::all_connections().into_iter().find(|c| c["connectionId"].as_str().map_or(false, |x| x.eq_ignore_ascii_case(cid))).and_then(|c| c["profileId"].as_str().map(|s| s.to_string()));
                match owner {
                    None => ubi_error(404, 1008, "No websocket connection is open with this connection id"),
                    Some(p) if !p.eq_ignore_ascii_case(&self.profile_id) => ubi_error(403, 1100, "The connection belongs to another profile"),
                    Some(_) => match store::update_connection(&self.profile_id, Some(cid), body()) {
                        Some(c) => Response::json(&c),
                        None => ubi_error(404, 1008, "No websocket connection is open with this connection id"),
                    },
                }
            }
            ("GET", ["profiles", "connections", cid]) => match store::all_connections().into_iter().find(|c| c["connectionId"].as_str().map_or(false, |x| x.eq_ignore_ascii_case(cid))) {
                Some(c) => Response::json(&c),
                None => ubi_error(404, 1008, "No websocket connection is open with this connection id"),
            },

            ("POST", ["profiles", target, "notifications"]) => Response::json(&self.notify(target, &body(), &self.space(req))),
            ("POST", ["profiles", "notifications"]) => {
                let b = body();
                let n = if b["notification"].is_null() { b.clone() } else { b["notification"].clone() };
                let targets: Vec<String> = list_of(&b["profileIds"]).iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect();
                let out: Vec<Value> = targets.iter().map(|t| self.notify(t, &n, &self.space(req))).collect();
                Response::json(&json!({ "notifications": out }))
            }

            ("POST", ["spaces", sid, "parties"]) => created(&self.st().create_party(sid, &self.profile_id, body())),
            ("GET", ["spaces", sid, "parties"]) => Response::json(&json!({ "parties": self.st().parties(sid) })),
            ("GET", ["profiles", pid, "parties"]) => {
                let owner = if self.me(pid) { self.profile_id.clone() } else { pid.to_ascii_lowercase() };
                Response::json(&json!({ "parties": self.st().parties_of(&owner) }))
            }
            ("GET", ["spaces", _, "parties", party]) => match self.st().party(party) {
                Some(p) => Response::json(&p),
                None => ubi_error(404, 1009, "Party not found"),
            },
            ("PUT" | "POST", ["spaces", _, "parties", party, "members", member]) => {
                let m = if self.me(member) { self.profile_id.clone() } else { member.to_ascii_lowercase() };
                match self.st().join_party(party, &m) {
                    Some(p) => Response::json(&p),
                    None => ubi_error(404, 1009, "Party not found"),
                }
            }
            ("DELETE", ["spaces", _, "parties", party, "members", member]) => {
                let m = if self.me(member) { self.profile_id.clone() } else { member.to_ascii_lowercase() };
                match self.st().leave_party(party, &m) {
                    Some(p) => Response::json(&p),
                    None => ubi_error(404, 1009, "Party not found"),
                }
            }
            ("PUT" | "POST", ["spaces", _, "parties", party, "xboxsync"]) => match self.st().sync_party(party, body()) {
                Some(p) => Response::json(&p),
                None => ubi_error(404, 1009, "Party not found"),
            },

            ("GET", ["profiles", pid, "gamesplayed"]) => {
                let owner = if self.me(pid) { self.profile_id.clone() } else { pid.to_ascii_lowercase() };
                Response::json(&json!({ "gamesPlayed": self.st().games_played(&owner) }))
            }
            ("PUT" | "POST", ["profiles", pid, "gamesplayed"]) if self.me(pid) => {
                let space = self.space(req);
                Response::json(&self.st().record_game_played(&self.profile_id, &space))
            }

            ("GET", ["profiles", "me", "news"]) | ("GET", ["spaces", "news"]) | ("GET", ["spaces", _, "news"]) => Response::json(&json!({ "news": self.st().list("news") })),

            ("GET" | "POST", ["spaces", _, "global", "tgdp", "gateway", "core", "remote_file", "get_upload_url"]) => {
                let id = random_uuid();
                let host = req.header("x-forwarded-host").unwrap_or("public-ubiservices.ubi.com");
                let url = format!("https://{host}/v1/tombox/upload/{id}");
                self.st().push("uploadUrls", json!({ "fileId": id, "url": url, "request": req.json() }));
                Response::json(&json!({ "url": url, "uploadUrl": url, "fileId": id }))
            }

            _ => return None,
        };
        Some(r)
    }
}

impl Default for XDefiant {
    fn default() -> Self {
        XDefiant::new()
    }
}

impl TitleService for XDefiant {
    fn name(&self) -> &'static str {
        "xdefiant"
    }

    fn route(&self, req: &Request) -> Option<Response> {
        let all = req.segments();
        let seg: Vec<&str> = match all.first() {
            Some(v) if (v.len() >= 2 && v.starts_with('v') && v[1..].bytes().all(|b| b.is_ascii_digit())) || *v == "{version}" => all[1..].to_vec(),
            _ => all.clone(),
        };
        let mut lower: Vec<String> = seg.iter().map(|s| s.to_string()).collect();
        for (i, s) in lower.iter_mut().enumerate() {
            if !(i > 0 && matches!(seg[i - 1], "profiles" | "spaces" | "entities" | "parties" | "members" | "friends" | "blocklist" | "seasons" | "upload" | "applications" | "users" | "connections" | "notifications")) {
                *s = s.to_ascii_lowercase();
            }
        }
        let norm: Vec<&str> = lower.iter().map(|s| s.as_str()).collect();
        let m = req.method.as_str();
        let r = self.route_v(req, m, &norm);
        if r.is_some() {
            self.st().count_request(&format!("{m} {}", norm.iter().map(|s| if is_uuid(s) { "{id}" } else { s }).collect::<Vec<_>>().join("/")));
        }
        r
    }

    fn not_found(&self, req: &Request) -> Response {
        ubi_error(404, 404, &format!("TomBox has no implementation of {} {} yet", req.method, req.path))
    }
}

pub struct Notifications {
    connection_id: String,
    profile_id: String,
    outlet: Option<Outlet>,
}

impl Notifications {
    pub fn new() -> Notifications {
        Notifications::for_profile(&uuid_for(&default_name()))
    }

    pub fn for_profile(profile_id: &str) -> Notifications {
        Notifications { connection_id: random_uuid(), profile_id: profile_id.to_ascii_lowercase(), outlet: None }
    }
}

impl Default for Notifications {
    fn default() -> Self {
        Notifications::new()
    }
}

pub fn connection_established(connection_id: &str) -> Value {
    json!({
        "notificationType": "US_CONNECTION_ESTABLISHED",
        "content": {
            "connectionId": connection_id,
            "contactUrl": format!("https://public-ubiservices.ubi.com/v1/profiles/connections/{connection_id}"),
        },
        "datePosted": iso_now(),
        "spaceId": DEFAULT_SPACE_ID,
    })
}

impl WsSession for Notifications {
    fn name(&self) -> &'static str {
        "ubi-notifications"
    }

    fn attach(&mut self, outlet: Outlet) {
        outlet.subscribe(&format!("ubi:{}", self.profile_id));
        self.outlet = Some(outlet);
    }

    fn on_open(&mut self) -> WsReply {
        store::open_connection(&self.profile_id, &self.connection_id);
        WsReply::one(connection_established(&self.connection_id).to_string())
    }

    fn on_text(&mut self, text: &str) -> WsReply {
        let v: Value = serde_json::from_str(text).unwrap_or(Value::Null);
        match v["notificationType"].as_str() {
            Some(_) => {
                let target = v["targetProfileId"].as_str().unwrap_or(&self.profile_id).to_ascii_lowercase();
                let mut env = v.clone();
                env["sourceProfileId"] = json!(self.profile_id);
                if env["datePosted"].is_null() {
                    env["datePosted"] = json!(iso_now());
                }
                let n = super::ws::publish(&format!("ubi:{target}"), &env.to_string());
                crate::record_raw("online", "websocket", &format!("[ubi-notifications {}] relayed {} to {target} ({n} sessions)", self.connection_id, env["notificationType"]));
            }
            None => {
                let head: String = text.chars().take(200).collect();
                crate::record_raw("online", "xdefiant-unhandled", &format!("websocket client message {head}"));
            }
        }
        WsReply::none()
    }

    fn on_close(&mut self) {
        store::close_connection(&self.connection_id);
    }
}

#[cfg(test)]
mod tests;
