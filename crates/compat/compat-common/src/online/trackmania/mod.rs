pub mod core;
pub mod live;
#[cfg(test)]
mod tests;

use super::xdefiant::{default_name, is_uuid, uuid_for, XDefiant};
use super::{Request, Response, TitleService};
use serde_json::Value;

pub const APP_ID: &str = "418273dd-b410-429a-9b3f-ccd1f79d9561";

pub fn host_exe_is_trackmania() -> bool {
    std::env::current_exe().ok().and_then(|p| p.file_name().map(|f| f.to_string_lossy().to_ascii_lowercase())).map_or(false, |f| f == "gbxgame.exe" || f.starts_with("trackmania"))
}

pub fn host_of(req: &Request) -> String {
    req.header("x-forwarded-host").or(req.header("host")).map(|h| h.split(':').next().unwrap_or("").to_ascii_lowercase()).unwrap_or_default()
}

fn is_nadeo_host(h: &str) -> bool {
    ["nadeo.online", "nadeo.live", "nadeo.club", "trackmania.com"].iter().any(|d| h == *d || h.ends_with(&format!(".{d}")))
}

fn is_core_host(h: &str) -> bool {
    h == "nadeo.online" || h.ends_with(".nadeo.online")
}

pub const NADEO_PARAMETER_GROUP: &str = "NadeoServicesParameters";
pub const MASTERSERVER_INIT_URL: &str = "https://prod.trackmania.core.nadeo.online/";

pub fn nadeo_parameter_group() -> Value {
    serde_json::json!({ "fields": { "masterserver_init_url": MASTERSERVER_INIT_URL }, "relatedPopulation": Value::Null })
}

fn wants_group(req: &Request, group: &str) -> bool {
    match req.query("parameterGroups").filter(|w| !w.trim().is_empty()) {
        None => true,
        Some(w) => w.split(',').any(|n| n.trim().eq_ignore_ascii_case(group)),
    }
}

fn is_parameters_path(req: &Request) -> bool {
    let seg = req.segments();
    let seg: Vec<&str> = if seg.first().map_or(false, |v| v.len() >= 2 && v.starts_with('v') && v[1..].bytes().all(|b| b.is_ascii_digit())) { seg[1..].to_vec() } else { seg };
    req.method == "GET" && seg.len() == 3 && matches!(seg[0], "spaces" | "applications") && seg[2].eq_ignore_ascii_case("parameters")
}

pub fn with_nadeo_parameters(req: &Request, r: Response) -> Response {
    if r.status != 200 || !is_parameters_path(req) || !wants_group(req, NADEO_PARAMETER_GROUP) {
        return r;
    }
    let Ok(mut v) = serde_json::from_slice::<Value>(&r.body) else { return r };
    let Some(p) = v.get_mut("parameters").and_then(Value::as_object_mut) else { return r };
    if p.contains_key(NADEO_PARAMETER_GROUP) {
        return r;
    }
    p.insert(NADEO_PARAMETER_GROUP.into(), nadeo_parameter_group());
    crate::record_raw("online", "trackmania", &format!("{} served with masterserver_init_url={MASTERSERVER_INIT_URL}", NADEO_PARAMETER_GROUP));
    Response::json(&v)
}

pub struct Trackmania {
    pub ubi: XDefiant,
    pub core: core::Core,
    pub live: live::Live,
}

fn is_live_host(h: &str) -> bool {
    h.starts_with("live-services.") || h.starts_with("club-services.") || h.starts_with("club.")
}

impl Trackmania {
    pub fn new() -> Trackmania {
        let space = crate::cfg("trackmania_space_id").filter(|s| is_uuid(s)).unwrap_or_else(|| uuid_for(&format!("trackmania-space:{APP_ID}")));
        Trackmania::with(&default_name(), &space)
    }

    pub fn with(name: &str, space: &str) -> Trackmania {
        let mut ubi = XDefiant::with(name, space);
        ubi.notifications = super::xdefiant::notifications_requested();
        let core = core::Core::new(name, &ubi.profile_id);
        let n = live::seed_maps(&core);
        if n > 0 {
            crate::record_raw("online", "trackmania", &format!("{n} campaign maps registered in Core from the live-services data"));
        }
        Trackmania { ubi, core, live: live::Live::new() }
    }

    fn nadeo(&self, req: &Request, host: &str) -> Option<Response> {
        let seg = core::segments_of(req);
        if req.method == "POST" && seg.len() == 4 && seg[0] == "v2" && seg[1] == "authentication" && seg[2] == "token" {
            return Some(self.core.token(seg[3], req, self.ubi.session_ticket()));
        }
        if is_core_host(host) {
            return Some(match core::match_route(&req.method, &seg) {
                Some((id, params)) => self.core.handle(id, &params, req, host),
                None => {
                    crate::record_raw("online", "unhandled", &format!("[trackmania] core {} {} {}", host, req.method, req.path));
                    core::nadeo_error(404, "C-AA-00-04", "Route not found.", Value::Null)
                }
            });
        }
        if is_live_host(host) {
            if let Some(r) = self.live.handle(&self.core, req) {
                return Some(r);
            }
        }
        crate::record_raw("online", "unhandled", &format!("[trackmania] {} {} {} (audience {})", host, req.method, req.path, core::audience_for_host(host)));
        None
    }
}

impl Default for Trackmania {
    fn default() -> Self {
        Trackmania::new()
    }
}

impl TitleService for Trackmania {
    fn name(&self) -> &'static str {
        "trackmania"
    }

    fn route(&self, req: &Request) -> Option<Response> {
        let host = host_of(req);
        if is_nadeo_host(&host) {
            return self.nadeo(req, &host);
        }
        let r = self.ubi.route(req)?;
        Some(with_nadeo_parameters(req, r))
    }

    fn not_found(&self, req: &Request) -> Response {
        let host = host_of(req);
        if is_nadeo_host(&host) {
            core::nadeo_error(404, "C-AA-00-04", &format!("TomBox has no implementation of {} {}{} yet", req.method, host, req.path), Value::Null)
        } else {
            self.ubi.not_found(req)
        }
    }
}
