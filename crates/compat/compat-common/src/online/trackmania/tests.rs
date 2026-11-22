use super::*;
use serde_json::{json, Value};

const CORE: &str = "prod.trackmania.core.nadeo.online";

fn req(method: &str, host: &str, path: &str, query: &[(&str, &str)], headers: &[(&str, &str)], body: &[u8]) -> Request {
    let mut h: Vec<(String, String)> = headers.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    h.push(("X-Forwarded-Host".into(), host.into()));
    Request { method: method.into(), path: path.into(), query: query.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(), headers: h, body: body.to_vec() }
}

fn body(r: &Response) -> Value {
    serde_json::from_slice(&r.body).unwrap_or(Value::Null)
}

fn tm() -> Trackmania {
    Trackmania::with("XboxEmulator", "11111111-2222-3333-4444-555555555555")
}

fn login(t: &Trackmania, audience: &str) -> (String, String) {
    let s = body(&t.route(&req("POST", "public-ubiservices.ubi.com", "/v3/profiles/sessions", &[], &[("Ubi-AppId", APP_ID)], b"{}")).unwrap());
    let ticket = s["ticket"].as_str().unwrap().to_string();
    let auth = format!("ubi_v1 t={ticket}");
    let r = t.route(&req("POST", CORE, "/v2/authentication/token/ubiservices", &[], &[("Authorization", &auth)], json!({"audience": audience}).to_string().as_bytes())).unwrap();
    assert_eq!(r.status, 200);
    let b = body(&r);
    (b["accessToken"].as_str().unwrap().into(), b["refreshToken"].as_str().unwrap().into())
}

fn claims(jwt: &str) -> Value {
    let p = jwt.split('.').nth(1).unwrap();
    let pad = match p.len() % 4 { 2 => "==", 3 => "=", _ => "" };
    let std_b64 = p.replace('-', "+").replace('_', "/") + pad;
    serde_json::from_slice(&crate::online::util::b64_decode(&std_b64)).unwrap()
}

#[test]
fn ubi_ticket_exchanges_for_nadeo_tokens() {
    let t = tm();
    let bad = t.route(&req("POST", CORE, "/v2/authentication/token/ubiservices", &[], &[("Authorization", "ubi_v1 t=nope")], b"")).unwrap();
    assert_eq!(bad.status, 401);
    let (at, rt) = login(&t, "NadeoLiveServices");
    let a = claims(&at);
    assert_eq!(a["aud"], "NadeoLiveServices");
    assert_eq!(a["sub"], json!(t.core.account_id));
    assert!(a["exp"].as_u64().unwrap() > a["rat"].as_u64().unwrap());
    let r = claims(&rt);
    assert_eq!(r["aud"], "NadeoServices");
    assert_eq!(r["refresh_aud"], "NadeoLiveServices");
    let auth = format!("nadeo_v1 t={rt}");
    let n = body(&t.route(&req("POST", CORE, "/v2/authentication/token/refresh", &[], &[("Authorization", &auth)], b"")).unwrap());
    assert_eq!(claims(n["accessToken"].as_str().unwrap())["aud"], "NadeoLiveServices");
    let again = t.route(&req("POST", CORE, "/v2/authentication/token/refresh", &[], &[("Authorization", &auth)], b"")).unwrap();
    assert_eq!(again.status, 401);
    let wrong = t.route(&req("POST", CORE, "/v2/authentication/token/ubiservices", &[], &[("Authorization", "ubi_v1 t=x")], br#"{"audience":"Bogus"}"#)).unwrap();
    assert_eq!(wrong.status, 401);
}

#[test]
fn routes_table_and_matching() {
    let t = tm();
    let ok = t.route(&req("GET", CORE, "/api/routes", &[("usage", "Client")], &[], b"")).unwrap();
    assert_eq!(ok.status, 200);
    let table = body(&ok);
    assert!(table.as_object().unwrap().len() >= 100);
    assert_eq!(t.route(&req("GET", CORE, "/api/routes", &[("usage", "Other")], &[], b"")).unwrap().status, 400);
    let (id, p) = core::match_route("GET", &["accounts", "abc", "zone"]).unwrap();
    assert!(id.contains("zone"), "{id}");
    assert_eq!(p["accountId"], "abc");
    assert!(core::match_route("GET", &["zones"]).is_some());
}

#[test]
fn account_state_round_trips() {
    let t = tm();
    let (at, _) = login(&t, "NadeoServices");
    let auth = format!("nadeo_v1 t={at}");
    let me = t.core.account_id.clone();
    let h = [("Authorization", auth.as_str()), ("Content-Type", "application/json")];
    assert_eq!(t.route(&req("GET", CORE, "/zones/", &[], &[], b"")).unwrap().status, 401);
    let zones = body(&t.route(&req("GET", CORE, "/zones/", &[], &h, b"")).unwrap());
    let france = zones.as_array().unwrap().iter().find(|z| z["name"] == "France").unwrap()["zoneId"].clone();
    let set = t.route(&req("PUT", CORE, &format!("/accounts/{me}/zone"), &[], &h, json!({"zoneId": france}).to_string().as_bytes())).unwrap();
    assert_eq!(set.status, 200, "{}", String::from_utf8_lossy(&set.body));
    let got = body(&t.route(&req("GET", CORE, &format!("/accounts/{me}/zone"), &[], &h, b"")).unwrap());
    assert_eq!(got["zoneId"], france);
    let other = "00000000-0000-0000-0000-000000000001";
    assert_eq!(t.route(&req("PUT", CORE, &format!("/accounts/{other}/zone"), &[], &h, json!({"zoneId": france}).to_string().as_bytes())).unwrap().status, 403);
    t.route(&req("PUT", CORE, &format!("/accounts/{me}/clubTag"), &[], &h, br#"{"clubTag":"$f00TB"}"#)).unwrap();
    let tags = body(&t.route(&req("GET", CORE, "/accounts/clubTags/", &[("accountIdList", &me)], &h, b"")).unwrap());
    assert_eq!(tags[0]["clubTag"], "$f00TB");
}

#[test]
fn map_upload_and_records() {
    let t = tm();
    let (at, _) = login(&t, "NadeoServices");
    let auth = format!("nadeo_v1 t={at}");
    let boundary = "XB";
    let mut mp = Vec::new();
    for (k, v) in [("mapUid", "uid123"), ("name", "Test"), ("authorScore", "30000"), ("goldScore", "32000"), ("silverScore", "36000"), ("bronzeScore", "45000"), ("isPlayable", "true")] {
        mp.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{k}\"\r\n\r\n{v}\r\n").as_bytes());
    }
    mp.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"data\"; filename=\"Test.Map.Gbx\"\r\nContent-Type: application/octet-stream\r\n\r\nGBXDATA\r\n--{boundary}--\r\n").as_bytes());
    let ct = format!("multipart/form-data; boundary={boundary}");
    let r = t.route(&req("POST", CORE, "/maps/", &[], &[("Authorization", &auth), ("Content-Type", &ct)], &mp)).unwrap();
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let m = body(&r);
    assert_eq!(m["mapUid"], "uid123");
    let url = m["fileUrl"].as_str().unwrap();
    let sid = url.rsplit('/').next().unwrap();
    let file = t.route(&req("GET", CORE, &format!("/storageObjects/{sid}"), &[], &[], b"")).unwrap();
    assert_eq!(file.body, b"GBXDATA");
    let list = body(&t.route(&req("GET", CORE, "/maps/", &[("mapUidList", "uid123")], &[("Authorization", &auth)], b"")).unwrap());
    assert_eq!(list.as_array().unwrap().len(), 1);
    let h = [("Authorization", auth.as_str()), ("Content-Type", "application/json")];
    let mid = m["mapId"].as_str().unwrap();
    let rec = |time: i64| body(&t.route(&req("POST", CORE, "/mapRecords/", &[], &h, json!({"mapId": mid, "recordScore": {"time": time, "respawnCount": 0, "score": 0}}).to_string().as_bytes())).unwrap());
    let first = rec(33000);
    assert_eq!(first["medal"], 2);
    assert_eq!(rec(40000)["recordScore"]["time"], 33000);
    assert_eq!(rec(29000)["medal"], 4);
    let all = body(&t.route(&req("GET", CORE, "/mapRecords/", &[("mapIdList", mid)], &[("Authorization", &auth)], b"")).unwrap());
    assert_eq!(all.as_array().unwrap().len(), 1);
}

#[test]
fn unknown_route_is_reported_not_faked() {
    let t = tm();
    let r = t.route(&req("GET", CORE, "/definitely/not/a/route", &[], &[], b"")).unwrap();
    assert_eq!(r.status, 404);
    assert!(body(&r)["correlation_id"].is_string());
}

#[test]
fn live_services_solo_flow() {
    const LIVE: &str = "live-services.trackmania.nadeo.live";
    let t = tm();
    let (core_at, _) = login(&t, "NadeoServices");
    let core_auth = format!("nadeo_v1 t={core_at}");
    assert_eq!(t.route(&req("GET", LIVE, "/api/token/club/player/info", &[], &[("Authorization", &core_auth)], b"")).unwrap().status, 401);
    let (at, _) = login(&t, "NadeoLiveServices");
    let auth = format!("nadeo_v1 t={at}");
    let h = [("Authorization", auth.as_str()), ("Content-Type", "application/json")];
    let get = |path: &str, q: &[(&str, &str)]| t.route(&req("GET", LIVE, path, q, &h, b"")).unwrap();
    let post = |path: &str, q: &[(&str, &str)], b: Value| t.route(&req("POST", LIVE, path, q, &h, b.to_string().as_bytes())).unwrap();
    let camp = body(&get("/api/token/campaign/official", &[("offset", "0"), ("length", "1")]));
    assert!(camp["campaignList"].is_array() && camp["relativeNextRequest"].as_i64().unwrap() > 0, "{camp}");
    let totd = body(&get("/api/token/campaign/month", &[("length", "1"), ("offset", "0")]));
    assert!(totd["monthList"].is_array());
    assert!(body(&get("/api/token/advertising/display/active", &[]))["displayList"].is_array());
    assert_eq!(body(&get("/api/token/club/player/info", &[]))["pinnedClub"], 0);
    assert_eq!(get("/api/token/map/nope", &[]).status, 404);

    let boundary = "XB";
    let mut mp = Vec::new();
    for (k, v) in [("mapUid", "solo1"), ("name", "Solo"), ("authorScore", "30000"), ("goldScore", "32000"), ("silverScore", "36000"), ("bronzeScore", "45000"), ("isPlayable", "true")] {
        mp.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{k}\"\r\n\r\n{v}\r\n").as_bytes());
    }
    mp.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"data\"; filename=\"Solo.Map.Gbx\"\r\n\r\nGBX\r\n--{boundary}--\r\n").as_bytes());
    let ct = format!("multipart/form-data; boundary={boundary}");
    let m = body(&t.route(&req("POST", CORE, "/maps/", &[], &[("Authorization", &core_auth), ("Content-Type", &ct)], &mp)).unwrap());
    let ch = [("Authorization", core_auth.as_str()), ("Content-Type", "application/json")];
    t.route(&req("POST", CORE, "/mapRecords/", &[], &ch, json!({"mapId": m["mapId"], "recordScore": {"time": 31000, "respawnCount": 0, "score": 0}}).to_string().as_bytes())).unwrap();

    let info = body(&get("/api/token/map/solo1", &[]));
    assert_eq!(info["authorTime"], 30000);
    assert_eq!(info["favorite"], false);
    assert_eq!(post("/api/token/map/favorite/solo1/add", &[], json!({})).status, 200);
    let favs = body(&get("/api/token/map/favorite", &[("offset", "0"), ("length", "10")]));
    assert_eq!(favs["itemCount"], 1);
    assert_eq!(favs["mapList"][0]["favorite"], true);
    assert_eq!(body(&get("/api/token/map", &[]))["itemCount"], 1);
    assert_eq!(body(&get("/api/token/map/get-multiple", &[("mapUidList", "solo1,other")]))["mapList"].as_array().unwrap().len(), 1);

    let recs = body(&post("/api/token/leaderboard/group/map", &[], json!({"maps": [{"mapUid": "solo1", "groupUid": "Personal_Best"}, {"mapUid": "none", "groupUid": "Personal_Best"}]})));
    assert_eq!(recs.as_array().unwrap().len(), 1, "{recs}");
    assert_eq!(recs[0]["score"], 31000);
    assert_eq!(recs[0]["zones"][0]["zoneName"], "World");
    let pos = body(&post("/api/token/leaderboard/group/map", &[("scores[solo1]", "35000")], json!({"maps": [{"mapUid": "solo1", "groupUid": "Personal_Best"}]})));
    assert_eq!(pos[0]["zones"][0]["ranking"]["position"], 2);
    let top = body(&get("/api/token/leaderboard/group/Personal_Best/map/solo1/top", &[("length", "5"), ("offset", "0")]));
    assert_eq!(top["tops"][0]["top"][0]["score"], 31000);
    let medals = body(&get("/api/token/leaderboard/group/Personal_Best/map/solo1/medals", &[]));
    assert_eq!(medals["medals"].as_array().unwrap().iter().map(|m| m["medal"].as_str().unwrap()).collect::<Vec<_>>(), vec!["Gold", "Silver", "Bronze"]);
    assert_eq!(body(&get("/api/token/leaderboard/trophy", &[]))["countPoint"], 0);

    let club = body(&post("/api/token/club/create", &[], json!({"name": "Local", "tag": "LOC", "state": "private-closed"})));
    let cid = club["id"].as_u64().unwrap();
    assert_eq!(body(&get("/api/token/club/mine", &[]))["clubList"][0]["role"], "Creator");
    assert_eq!(body(&post(&format!("/api/token/club/{cid}/pin"), &[], json!({})))["pinnedClub"], cid);
    assert_eq!(body(&get("/api/token/club/player/info", &[]))["pinnedClubName"], "Local");
    let cc = body(&post(&format!("/api/token/club/{cid}/campaign/create"), &[], json!({"name": "Mine", "playlist": [{"mapUid": "solo1", "position": 0}]})));
    let campaign_id = cc["campaign"]["id"].as_u64().unwrap();
    let got = body(&get(&format!("/api/token/club/{cid}/campaign/{campaign_id}"), &[]));
    assert_eq!(got["campaign"]["playlist"][0]["mapUid"], "solo1");
    let group = got["campaign"]["leaderboardGroupUid"].as_str().unwrap().to_string();
    assert_eq!(body(&get(&format!("/api/token/leaderboard/group/{group}"), &[]))["sp"], "1000");
    assert_eq!(body(&get(&format!("/api/token/club/{cid}/activity"), &[]))["itemCount"], 1);
    assert_eq!(post(&format!("/api/token/club/{cid}/delete"), &[], json!({})).status, 200);
    assert_eq!(get(&format!("/api/token/club/{cid}"), &[]).status, 404);
    assert!(t.route(&req("GET", LIVE, "/api/token/club/room", &[], &h, b"")).is_none());
}

#[test]
fn seeded_campaign_map_resolves_in_core_and_live() {
    let t = tm();
    let m = t.core.seed_map(&json!({"mapUid": "camp01", "name": "Campaign 01", "authorScore": 20000, "goldScore": 22000, "silverScore": 25000, "bronzeScore": 30000}), b"MAPFILE".to_vec()).unwrap();
    let (at, _) = login(&t, "NadeoServices");
    let auth = format!("nadeo_v1 t={at}");
    let list = body(&t.route(&req("GET", CORE, "/maps/", &[("mapUidList", "camp01")], &[("Authorization", &auth)], b"")).unwrap());
    assert_eq!(list[0]["mapId"], m["mapId"]);
    let sid = m["fileUrl"].as_str().unwrap().rsplit('/').next().unwrap().to_string();
    assert_eq!(t.route(&req("GET", CORE, &format!("/storageObjects/{sid}"), &[], &[], b"")).unwrap().body, b"MAPFILE");
    let (lt, _) = login(&t, "NadeoLiveServices");
    let la = format!("nadeo_v1 t={lt}");
    let info = body(&t.route(&req("GET", "live-services.trackmania.nadeo.live", "/api/token/map/camp01", &[], &[("Authorization", &la)], b"")).unwrap());
    assert_eq!(info["goldTime"], 22000);
    assert_eq!(t.core.seed_map(&json!({"mapUid": "camp01"}), vec![]).unwrap()["mapId"], m["mapId"]);
}

#[test]
fn space_parameters_carry_nadeo_masterserver_url() {
    let t = tm();
    let space = t.ubi.space_id.clone();
    let groups = "us-sdkClientFeaturesSwitches,us-sdkClientUrls,NadeoServicesParameters,UbiConnect";
    let r = t.route(&req("GET", "public-ubiservices.ubi.com", &format!("/v1/spaces/{space}/parameters"), &[("parameterGroups", groups)], &[], b"")).unwrap();
    assert_eq!(r.status, 200);
    let b = body(&r);
    let url = b["parameters"]["NadeoServicesParameters"]["fields"]["masterserver_init_url"].as_str().unwrap();
    assert!(url.starts_with("https://") && url.trim_end_matches('/').ends_with(CORE));
    assert!(b["parameters"]["us-sdkClientUrls"].is_object());
    let other = body(&t.route(&req("GET", "public-ubiservices.ubi.com", &format!("/v1/spaces/{space}/parameters"), &[("parameterGroups", "us-sdkClientUrls")], &[], b"")).unwrap());
    assert!(other["parameters"].get("NadeoServicesParameters").is_none());
    let host = url.split("://").nth(1).unwrap().split('/').next().unwrap();
    let routes = t.route(&req("GET", host, "/api/routes", &[("usage", "Client")], &[], b"")).unwrap();
    assert_ne!(routes.status, 404, "the masterserver host must serve Nadeo core routes");
}

#[test]
fn contact_url_patch_updates_the_open_connection() {
    let t = tm();
    let cid = crate::online::util::random_uuid();
    crate::online::xdefiant::store::open_connection(&t.ubi.profile_id, &cid);
    let r = t.route(&req("PATCH", "public-ubiservices.ubi.com", &format!("/v1/profiles/connections/{cid}"), &[], &[], br#"{"notificationTypes":["US_CONNECTION_ESTABLISHED"],"spaceIds":["72c7e441-cf95-56e4-a2b3-3abed655b33a"]}"#)).unwrap();
    assert_eq!(r.status, 200);
    let c = body(&r);
    assert_eq!(c["connectionId"].as_str().unwrap(), cid);
    assert_eq!(c["notificationTypes"][0], "US_CONNECTION_ESTABLISHED");
    assert_eq!(body(&t.route(&req("GET", "public-ubiservices.ubi.com", &format!("/v1/profiles/connections/{cid}"), &[], &[], b"")).unwrap())["spaceIds"][0], "72c7e441-cf95-56e4-a2b3-3abed655b33a");
    let foreign = crate::online::util::random_uuid();
    crate::online::xdefiant::store::open_connection("99999999-9999-9999-9999-999999999999", &foreign);
    assert_eq!(t.route(&req("PATCH", "public-ubiservices.ubi.com", &format!("/v1/profiles/connections/{foreign}"), &[], &[], b"{}")).unwrap().status, 403);
    let gone = t.route(&req("PATCH", "public-ubiservices.ubi.com", "/v1/profiles/connections/00000000-0000-0000-0000-000000000000", &[], &[], b"{}")).unwrap();
    assert_eq!(gone.status, 404);
    crate::online::xdefiant::store::close_connection(&cid);
    crate::online::xdefiant::store::close_connection(&foreign);
}
