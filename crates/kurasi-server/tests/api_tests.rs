//! Every API route on a temporary project (a port of tests/test_api.py), driven through the
//! router with `tower::ServiceExt::oneshot`.

mod common;

use std::time::{Duration, Instant};

use common::{folder, home, make_pdf, ref_by_slug, test_app, ui_dir, Client};
use kurasi_core::model::{HighlightFilter, Origin};
use serde_json::{json, Value};

fn keys(v: &Value) -> Vec<String> {
    v.as_object().map(|o| o.keys().cloned().collect()).unwrap_or_default()
}

fn has_keys(v: &Value, wanted: &[&str]) -> bool {
    let k = keys(v);
    wanted.iter().all(|w| k.iter().any(|x| x == w))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn static_and_host_checks() {
    let h = home();
    let ui = ui_dir(h.dir.path());
    let (app, _ws, _) = test_app(&h.paths, &ui);
    let c = Client::new(app);
    let r = c.get("/").await;
    assert_eq!(r.status, 200);
    assert!(String::from_utf8_lossy(&r.bytes).contains("Kurasi test"));
    assert!(r.ctype.starts_with("text/html"));
    assert_eq!(c.get("/ui/app.css").await.status, 200);
    assert_eq!(c.get("/app.css").await.status, 200);
    assert_eq!(c.get("/ui/assets/splash.jpg").await.ctype, "image/jpeg");
    assert_eq!(c.get("/ui/../kurasi/api.py").await.status, 404);
    assert_eq!(c.get("/nothing.js").await.status, 404);
    assert_eq!(c.with_host("localhost:7343").get("/api/state").await.status, 200);
    let r = c.with_host("evil.example:7343").get("/api/state").await;
    assert_eq!(r.status, 403);
    assert!(!r.err().is_empty());
    assert_eq!(c.with_origin("http://evil.example").get("/api/state").await.status, 403);
    assert_eq!(c.with_origin("http://127.0.0.1:7343").get("/api/state").await.status, 200);
    let r = c.get("/api/nope").await;
    assert_eq!(r.status, 404);
    assert!(r.err().contains("/api/nope"));
    assert_eq!(c.request("PATCH", "/api/state", None).await.status, 405);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn state_without_project() {
    let h = home();
    let ui = ui_dir(h.dir.path());
    let (app, _ws, _) = test_app(&h.paths, &ui);
    let c = Client::new(app);
    let r = c.get("/api/state").await;
    assert_eq!(r.status, 200);
    assert!(r.json["project"].is_null());
    assert_eq!(r.json["demo"], json!(false));
    assert_eq!(r.json["change_seq"], json!(0));
    assert_eq!(r.json["startup"], json!({"opened_last": false, "folder_missing": false, "missing_name": null, "missing_folder": null}));
    let r = c.get("/api/references").await;
    assert_eq!(r.status, 400);
    assert!(r.err().contains("No project is open"));
    assert_eq!(c.get("/api/scan/status").await.json["running"], json!(false));
    assert_eq!(c.get("/api/changes?since=0").await.json, json!({"seq": 0, "changes": []}));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn project_create_scan_and_state() {
    let h = home();
    let ui = ui_dir(h.dir.path());
    let proj = folder(h.dir.path());
    let (app, _ws, _) = test_app(&h.paths, &ui);
    let c = Client::new(app);
    let opened = c.open_project(&proj).await;
    assert_eq!(opened["project"]["name"], json!("Test"));
    let st = c.get("/api/state").await.json;
    assert_eq!(st["project"]["folder"], json!(proj.to_string_lossy()));
    assert_eq!(st["project"]["auto_scan_min"], json!(0));
    assert_eq!(st["counts"]["references"], json!(7));
    assert_eq!(st["projects"][0]["exists"], json!(true));
    let scan = c.get("/api/scan/status").await.json;
    assert!(scan["result"]["new"].as_i64().unwrap() >= 8);
    assert_eq!(scan["pct"], json!(100));
    let again = c.post("/api/scan", json!({"full": true})).await;
    assert_eq!(again.status, 200);
    assert!(again.json.get("started").is_some());
    let st = c.wait_scan().await;
    assert_eq!(st["result"]["full"], json!(true));
    let missing = format!("{}-missing", proj.to_string_lossy());
    assert_eq!(c.post("/api/project", json!({"folder": missing})).await.status, 400);
    assert_eq!(c.post("/api/project", json!({})).await.status, 400);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn references_crud_undo_and_categories() {
    let h = home();
    let ui = ui_dir(h.dir.path());
    let proj = folder(h.dir.path());
    let (app, _ws, _) = test_app(&h.paths, &ui);
    let c = Client::new(app);
    c.open_project(&proj).await;
    let r = c.refs().await["smith_1776_wealth"].clone();
    assert!(has_keys(
        &r,
        &[
            "id", "title", "authors", "kind", "pub", "year", "date", "lang", "usability", "status", "register_id", "categories", "ext",
            "pages", "hl_count", "top_tags", "missing", "files", "primary_file_id", "thumb_url", "created_at", "updated_at", "search_text",
        ]
    ));
    assert!(r["search_text"].as_str().unwrap().contains("division of labour"));
    let id = r["id"].as_str().unwrap().to_string();
    let detail = c.get(&format!("/api/reference/{}", id)).await;
    assert_eq!(detail.status, 200);
    assert!(has_keys(&detail.json, &["doi", "url", "vol", "issue", "pages_range"]));
    let upd = c
        .put(
            &format!("/api/reference/{}", id),
            json!({"title": "An inquiry", "authors": ["Smith, A."], "kind": "Book", "pub": "Strahan", "doi": "10.1/x",
                   "categories": ["Economics/Classics", "Notes"], "usability": "Full text"}),
        )
        .await;
    assert_eq!(upd.status, 200, "{}", upd.err());
    assert_eq!(upd.json["status"], json!("Verified"));
    assert_eq!(upd.json["doi"], json!("10.1/x"));
    assert_eq!(upd.json["categories"], json!(["Economics/Classics", "Notes"]));
    assert_eq!(c.put(&format!("/api/reference/{}", id), json!({"usability": "Bad"})).await.status, 400);
    assert_eq!(c.put("/api/reference/r-000000000000", json!({"title": "x"})).await.status, 404);
    let upd = c.put(&format!("/api/reference/{}", id), json!({"status": "Suggested"})).await;
    assert_eq!(upd.json["status"], json!("Suggested"));

    let cats = c.get("/api/categories").await.json;
    let econ = cats.as_array().unwrap().iter().find(|x| x["name"] == json!("Economics")).unwrap().clone();
    assert_eq!(econ["children"][0]["path"], json!("Economics/Classics"));
    let new = c.post("/api/categories", json!({"parent_id": econ["id"], "name": "Modern"})).await;
    assert_eq!(new.status, 200, "{}", new.err());
    assert_eq!(new.json["path"], json!("Economics/Modern"));
    assert_eq!(c.post("/api/categories", json!({"parent_id": econ["id"], "name": "Modern"})).await.status, 409);
    assert_eq!(c.post("/api/categories", json!({"name": "a/b"})).await.status, 400);
    let moved = c.put(&format!("/api/categories/{}", new.json["id"]), json!({"name": "Recent", "parent_id": null})).await;
    assert_eq!(moved.json["path"], json!("Recent"));
    assert_eq!(
        c.put(&format!("/api/categories/{}", econ["id"]), json!({"parent_id": econ["children"][0]["id"]})).await.status,
        400
    );
    assert_eq!(c.put("/api/categories/1", json!({"name": "x"})).await.status, 400);
    let classics = econ["children"][0]["id"].as_i64().unwrap();
    let out = c.delete(&format!("/api/categories/{}", classics)).await;
    assert_eq!(out.status, 200);
    assert_eq!(out.json["moved"], json!(1));
    let mut cats_now: Vec<String> = c.refs().await["smith_1776_wealth"]["categories"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    cats_now.sort();
    assert_eq!(cats_now, vec!["Economics", "Notes"]);
    assert_eq!(c.delete("/api/categories/1").await.status, 400);

    let out = c.delete(&format!("/api/reference/{}", id)).await;
    assert_eq!(out.status, 200);
    let tid = out.json["trash_id"].as_i64().unwrap();
    assert!(tid > 0);
    assert!(!c.refs().await.contains_key("smith_1776_wealth"));
    let res = c.post("/api/undo", json!({"trash_id": tid})).await;
    assert_eq!(res.json, json!({"restored": "reference", "id": id}));
    assert!(c.refs().await.contains_key("smith_1776_wealth"));
    assert_eq!(c.post("/api/undo", json!({"trash_id": 99999})).await.status, 404);
    assert_eq!(c.post("/api/undo", json!({})).await.status, 400);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn authors_and_tags() {
    let h = home();
    let ui = ui_dir(h.dir.path());
    let proj = folder(h.dir.path());
    let (app, _ws, _) = test_app(&h.paths, &ui);
    let c = Client::new(app);
    c.open_project(&proj).await;
    let authors = c.get("/api/authors").await;
    assert_eq!(authors.status, 200);
    let list = authors.json.as_array().unwrap();
    let smith = list.iter().find(|a| a["name"] == json!("Smith, A.")).unwrap();
    assert_eq!(smith["hl_count"], json!(3));
    assert!(smith["ref_ids"].as_array().unwrap().contains(&smith["top_ref_id"]));
    assert_eq!(list.last().unwrap()["name"], json!("Unknown"));
    let tags = c.get("/api/tags").await.json;
    let names: std::collections::HashMap<String, Value> =
        tags.as_array().unwrap().iter().map(|t| (t["name"].as_str().unwrap().to_string(), t.clone())).collect();
    for t in ["division-of-labour", "quote", "efficiency"] {
        assert!(names.contains_key(t), "tag {} missing", t);
    }
    assert_eq!(names["quote"]["hl_count"], json!(1));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn file_routes() {
    let h = home();
    let ui = ui_dir(h.dir.path());
    let proj = folder(h.dir.path());
    let (app, _ws, _) = test_app(&h.paths, &ui);
    let c = Client::new(app);
    c.open_project(&proj).await;
    let r = c.refs().await;
    let pdf = r["smith_1776_wealth"]["primary_file_id"].as_str().unwrap().to_string();
    let info = c.get(&format!("/api/file/{}/info", pdf)).await.json;
    assert_eq!(info["ext"], json!("pdf"));
    assert_eq!(info["pages"], json!(3));
    assert_eq!(info["page_sizes"][0], json!([595.0, 842.0]));
    assert_eq!(info["rotation"], json!([0, 0, 0]));
    let png = c.get(&format!("/api/file/{}/page/1.png?scale=1.5", pdf)).await;
    assert_eq!(png.status, 200, "{}", png.err());
    assert_eq!(png.ctype, "image/png");
    assert_eq!(&png.bytes[..4], b"\x89PNG");
    assert_eq!(c.get(&format!("/api/file/{}/page/1.png?scale=1.5", pdf)).await.bytes, png.bytes);
    assert_eq!(c.get(&format!("/api/file/{}/page/9.png", pdf)).await.status, 404);
    assert_eq!(c.get(&format!("/api/file/{}/page/one.png", pdf)).await.status, 404);
    let words = c.get(&format!("/api/file/{}/page/1/words", pdf)).await.json;
    assert_eq!(words["width"], json!(595.0));
    assert_eq!(words["rotation"], json!(0));
    assert_eq!(words["words"][0][4], json!("The"));
    let raw = c.get(&format!("/api/file/{}/raw", pdf)).await;
    assert_eq!(&raw.bytes[..4], b"%PDF");
    assert_eq!(raw.ctype, "application/pdf");
    let thumb = c.get(&format!("/api/thumb/{}.png", pdf)).await;
    assert_eq!(thumb.status, 200, "{}", thumb.err());
    assert_eq!(&thumb.bytes[..4], b"\x89PNG");
    let md = r["reading"]["primary_file_id"].as_str().unwrap().to_string();
    let raw = c.get(&format!("/api/file/{}/raw", md)).await;
    assert!(raw.ctype.starts_with("text/markdown"));
    assert!(String::from_utf8_lossy(&raw.bytes).contains("# Notes"));
    assert_eq!(c.get(&format!("/api/thumb/{}.png", md)).await.status, 404);
    let info = c.get(&format!("/api/file/{}/info", md)).await.json;
    assert_eq!(info["pages"], json!(0));
    assert_eq!(info["rotation"], json!([0]));
    let img = r["chart"]["primary_file_id"].as_str().unwrap().to_string();
    let info = c.get(&format!("/api/file/{}/info", img)).await.json;
    assert_eq!(info["page_sizes"], json!([[300, 200]]));
    assert_eq!(&c.get(&format!("/api/thumb/{}.png", img)).await.bytes[..4], b"\x89PNG");
    assert_eq!(c.get("/api/file/f-000000000000/info").await.status, 404);

    let info = c.post(&format!("/api/file/{}/rotate", pdf), json!({"page": 2, "delta": 90})).await;
    assert_eq!(info.status, 200, "{}", info.err());
    assert_eq!(info.json["rotation"], json!([0, 90, 0]));
    let info = c.post(&format!("/api/file/{}/rotate", pdf), json!({"page": null, "delta": 90})).await.json;
    assert_eq!(info["rotation"], json!([90, 180, 90]));
    let info = c.post(&format!("/api/file/{}/rotate", img), json!({"page": null, "delta": 90})).await.json;
    assert_eq!(info["rotation"], json!([90]));
    assert_eq!(c.post(&format!("/api/file/{}/rotate", img), json!({"delta": 45})).await.status, 400);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn highlight_routes() {
    let h = home();
    let ui = ui_dir(h.dir.path());
    let proj = folder(h.dir.path());
    let (app, _ws, _) = test_app(&h.paths, &ui);
    let c = Client::new(app);
    c.open_project(&proj).await;
    let r = c.refs().await;
    let pdf = r["smith_1776_wealth"]["primary_file_id"].as_str().unwrap().to_string();
    let smith_id = r["smith_1776_wealth"]["id"].as_str().unwrap().to_string();
    let words = c.get(&format!("/api/file/{}/page/3/words", pdf)).await.json["words"].clone();
    let (w0, w1) = (&words[0], &words[1]);
    let hl = c
        .post(
            "/api/highlights",
            json!({"file_id": pdf, "page": 3, "kind": "text", "quads": [[w0[0], w0[1], w1[2], w1[3]]],
                   "verbatim": "", "analysis": "(I) Mine.", "label": null, "tags": ["#method"]}),
        )
        .await;
    assert_eq!(hl.status, 200, "{}", hl.err());
    let hv = &hl.json;
    assert_eq!(
        (hv["author"].clone(), hv["origin"].clone(), hv["kind"].clone(), hv["verbatim"].clone(), hv["label"].clone(), hv["tags"].clone()),
        (json!("You"), json!("human"), json!("text"), json!("Measured tasks"), json!("I"), json!(["method"]))
    );
    let hid = hv["id"].as_str().unwrap().to_string();
    let hls = c.get(&format!("/api/highlights?ref_id={}", smith_id)).await.json;
    let ids: Vec<&str> = hls.as_array().unwrap().iter().map(|x| x["id"].as_str().unwrap()).collect();
    assert!(ids.contains(&hid.as_str()));
    assert_eq!(ids.len(), 4);
    assert!(has_keys(
        &hls[0],
        &[
            "id", "file_id", "ref_id", "page", "kind", "quads", "anchor", "verbatim", "analysis", "label", "tags", "author", "origin",
            "created_at", "updated_at", "suggestion", "suggested_tags",
        ]
    ));
    let h2 = c
        .put(&format!("/api/highlights/{}", hid), json!({"analysis": "(P) Changed.", "label": "P", "tags": ["method", "quote"]}))
        .await;
    assert_eq!(h2.status, 200, "{}", h2.err());
    assert_eq!(h2.json["analysis"], json!("(P) Changed."));
    assert_eq!(h2.json["label"], json!("P"));
    assert_eq!(c.put(&format!("/api/highlights/{}", hid), json!({"label": "X"})).await.status, 400);
    // The scan sees no outside change after the app wrote the PDF.
    c.post("/api/scan", json!({})).await;
    assert_eq!(c.wait_scan().await["result"]["changed"], json!(0));

    let md = r["reading"]["primary_file_id"].as_str().unwrap().to_string();
    let mh = c
        .post(
            "/api/highlights",
            json!({"file_id": md, "kind": "md", "anchor": {"text": "plain sentence", "occurrence": 0},
                   "verbatim": "plain sentence", "analysis": "(F) Md.", "tags": ["quote"]}),
        )
        .await;
    assert_eq!(mh.status, 200, "{}", mh.err());
    assert_eq!(mh.json["kind"], json!("md"));
    assert_eq!(mh.json["quads"], json!([]));
    assert_eq!(mh.json["anchor"]["occurrence"], json!(0));
    let img = r["chart"]["primary_file_id"].as_str().unwrap().to_string();
    let ih = c
        .post(
            "/api/highlights",
            json!({"file_id": img, "kind": "region", "quads": [[0.1, 0.2, 0.3, 0.4]], "verbatim": "Bars", "analysis": "(F) Image.", "tags": []}),
        )
        .await;
    assert_eq!(ih.status, 200, "{}", ih.err());
    assert_eq!(ih.json["quads"], json!([[0.1, 0.2, 0.3, 0.4]]));
    assert_eq!(c.post("/api/highlights", json!({"file_id": img, "quads": "x"})).await.status, 400);

    let out = c.delete(&format!("/api/highlights/{}", ih.json["id"].as_str().unwrap())).await;
    let tid = out.json["trash_id"].as_i64().unwrap();
    let back = c.post("/api/undo", json!({"trash_id": tid})).await;
    assert_eq!(back.json["restored"], json!("highlight"));
    let out = c.delete(&format!("/api/highlights/{}", hid)).await;
    assert_eq!(c.refs().await["smith_1776_wealth"]["hl_count"], json!(3));
    let back = c.post("/api/undo", json!({"trash_id": out.json["trash_id"]})).await;
    assert_eq!(back.status, 200, "{}", back.err());
    let restored = c.get(&format!("/api/highlights?file_id={}", pdf)).await.json;
    assert!(restored.as_array().unwrap().iter().any(|x| x["analysis"] == json!("(P) Changed.")));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn suggestions_and_accept_tag() {
    let h = home();
    let ui = ui_dir(h.dir.path());
    let proj = folder(h.dir.path());
    let (app, ws, _) = test_app(&h.paths, &ui);
    let c = Client::new(app);
    c.open_project(&proj).await;
    let p = ws.read().await.project.clone().unwrap();
    let smith = ref_by_slug(&p, "smith_1776_wealth");
    let hl = p.highlights(&HighlightFilter { ref_id: Some(smith.id.clone()), ..Default::default() }).unwrap().remove(0);
    p.suggest_tags(&hl.id, &["method".into(), "to-read".into()], Origin::Mcp).unwrap();
    let out = c.post(&format!("/api/highlights/{}/accept-tag", hl.id), json!({"tag": "method"})).await;
    assert_eq!(out.status, 200, "{}", out.err());
    assert!(out.json["tags"].as_array().unwrap().contains(&json!("method")));
    assert_eq!(out.json["suggested_tags"], json!(["to-read"]));
    let out = c.post(&format!("/api/highlights/{}/accept-tag", hl.id), json!({"tag": "to-read", "dismiss": true})).await;
    assert!(!out.json["tags"].as_array().unwrap().contains(&json!("to-read")));
    assert_eq!(out.json["suggested_tags"], json!([]));
    assert_eq!(c.post(&format!("/api/highlights/{}/suggestion", hl.id), json!({"resolution": "accept"})).await.status, 400);
    p.propose_delete(&hl.id, "Duplicate.", Origin::Mcp).unwrap();
    let out = c.post(&format!("/api/highlights/{}/suggestion", hl.id), json!({"resolution": "dismiss"})).await;
    assert_eq!(out.json["highlight"]["suggestion"]["resolution"], json!("dismissed"));
    p.propose_delete(&hl.id, "Duplicate again.", Origin::Mcp).unwrap();
    let out = c.post(&format!("/api/highlights/{}/suggestion", hl.id), json!({"resolution": "accept"})).await;
    assert_eq!(out.json["deleted"], json!(true));
    assert!(out.json["trash_id"].as_i64().unwrap() > 0);
    let left = c.get(&format!("/api/highlights?file_id={}", hl.file_id)).await.json;
    assert!(!left.as_array().unwrap().iter().any(|x| x["id"] == json!(hl.id)));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn settings_read_only_and_locked_writes() {
    let h = home();
    let ui = ui_dir(h.dir.path());
    let proj = folder(h.dir.path());
    let (app, _ws, _) = test_app(&h.paths, &ui);
    let c = Client::new(app);
    c.open_project(&proj).await;
    let s = c.put("/api/settings", json!({"read_only": true, "auto_scan_min": 5, "theme": "dark", "name": "Renamed"})).await;
    assert_eq!(s.status, 200, "{}", s.err());
    assert_eq!(s.json["read_only"], json!(true));
    assert_eq!(s.json["auto_scan_min"], json!(5));
    let st = c.get("/api/state").await.json;
    assert_eq!(st["project"]["name"], json!("Renamed"));
    assert_eq!(st["projects"][0]["name"], json!("Renamed"));
    assert_eq!(c.put("/api/settings", json!({"auto_scan_min": "x"})).await.status, 400);
    assert_eq!(c.put("/api/settings", json!({"folder": "C:\\elsewhere"})).await.status, 400);
    let r = c.refs().await;
    let pdf = r["smith_1776_wealth"]["primary_file_id"].as_str().unwrap().to_string();
    let body = c.post("/api/highlights", json!({"file_id": pdf, "page": 1, "quads": [[72, 72, 200, 90]], "analysis": "x"})).await;
    assert_eq!(body.status, 423);
    assert!(body.err().contains("read-only"));
    assert_eq!(c.post(&format!("/api/file/{}/rotate", pdf), json!({"page": 1})).await.status, 423);
    assert_eq!(c.post("/api/export", json!({"format": "apa-md"})).await.status, 423);
    assert_eq!(c.post("/api/ai-log/export", json!({})).await.status, 423);
    // Database writes still happen.
    let chart = r["chart"]["id"].as_str().unwrap();
    let upd = c.put(&format!("/api/reference/{}", chart), json!({"title": "Chart (read-only edit)"})).await;
    assert_eq!(upd.status, 200);
    assert_eq!(upd.json["title"], json!("Chart (read-only edit)"));
    c.put("/api/settings", json!({"read_only": false})).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn changes_ai_log_export_open_resolve_mcp() {
    let h = home();
    let ui = ui_dir(h.dir.path());
    let proj = folder(h.dir.path());
    let (app, ws, opened) = test_app(&h.paths, &ui);
    let c = Client::new(app);
    c.open_project(&proj).await;
    let ch = c.get("/api/changes?since=0").await.json;
    let seq = ch["seq"].as_i64().unwrap();
    assert!(seq >= 1);
    let first = ch["changes"][0]["origin"].as_str().unwrap();
    assert!(first == "scan" || first == "ui");
    let r = c.refs().await;
    let chart = r["chart"]["id"].as_str().unwrap().to_string();
    c.put(&format!("/api/reference/{}", chart), json!({"title": "Chart two"})).await;
    let ch = c.get(&format!("/api/changes?since={}", seq)).await.json;
    let origins: Vec<&str> = ch["changes"].as_array().unwrap().iter().map(|x| x["origin"].as_str().unwrap()).collect();
    assert_eq!(origins, vec!["ui"]);
    assert!(ch["changes"][0]["ids"].as_array().unwrap().contains(&json!(chart)));
    assert_eq!(c.get("/api/changes?since=x").await.status, 400);

    // The long-poll waits for the next change and answers at once when one arrives.
    let seq = ch["seq"].as_i64().unwrap();
    let t0 = Instant::now();
    let waited = c.get(&format!("/api/changes?since={}&timeout=1", seq)).await.json;
    assert!(waited["changes"].as_array().unwrap().is_empty());
    assert!(t0.elapsed() >= Duration::from_millis(900), "the long-poll answered early");
    let c2 = c.clone();
    let poke = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(200)).await;
        c2.put("/api/settings", json!({"theme": "light"})).await
    });
    let t0 = Instant::now();
    let woken = c.get(&format!("/api/changes?since={}&timeout=10", seq)).await.json;
    assert_eq!(woken["changes"].as_array().unwrap().len(), 1);
    assert!(t0.elapsed() < Duration::from_secs(5), "the long-poll missed the wake-up");
    poke.await.unwrap();

    let p = ws.read().await.project.clone().unwrap();
    p.log_ai("catalog_scan", "claude-code", "Rescan", &json!({"full": false}), "0 new", None).unwrap();
    p.log_ai("log_append", "claude-code", "Checked a quote", &json!({"checks": "Compared with page 3 | ok"}), "ok", None).unwrap();
    let log = c.get("/api/ai-log").await.json;
    let tools: Vec<&str> = log.as_array().unwrap().iter().map(|x| x["tool"].as_str().unwrap()).collect();
    assert_eq!(tools, vec!["log_append", "catalog_scan"]);
    let out = c.post("/api/ai-log/export", json!({})).await;
    assert_eq!(out.status, 200, "{}", out.err());
    let path = out.json["path"].as_str().unwrap().to_string();
    assert_eq!(path, proj.join(".kurasi").join("AI_USE_LOG.md").to_string_lossy());
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("| Date | Tool | Purpose | Checks performed by the author |"));
    assert!(text.contains("Compared with page 3 \\| ok"));

    let smith = r["smith_1776_wealth"]["id"].as_str().unwrap().to_string();
    for (fmt, name) in [
        ("apa-md", "references_apa.md"),
        ("bibtex", "references.bib"),
        ("csl-json", "references.csl.json"),
        ("highlights-json", "highlights.json"),
    ] {
        let out = c.post("/api/export", json!({"format": fmt, "ref_ids": [smith]})).await;
        assert_eq!(out.status, 200, "{}", out.err());
        assert_eq!(out.json["path"], json!(proj.join("_exports").join(name).to_string_lossy()));
        assert!(std::fs::metadata(out.json["path"].as_str().unwrap()).unwrap().len() > 10);
    }
    let apa = std::fs::read_to_string(proj.join("_exports/references_apa.md")).unwrap();
    assert!(apa.contains("Smith, A. (1776). *The wealth of nations*."), "{}", apa);
    let csl: Value = serde_json::from_str(&std::fs::read_to_string(proj.join("_exports/references.csl.json")).unwrap()).unwrap();
    assert_eq!(csl[0]["author"], json!([{"family": "Smith", "given": "A."}]));
    let hl: Value = serde_json::from_str(&std::fs::read_to_string(proj.join("_exports/highlights.json")).unwrap()).unwrap();
    assert_eq!(hl["highlights"].as_array().unwrap().len(), 3);
    assert_eq!(c.post("/api/export", json!({"format": "docx"})).await.status, 400);

    let fid = r["chart"]["primary_file_id"].as_str().unwrap().to_string();
    assert_eq!(c.post("/api/open", json!({"file_id": fid})).await.status, 200);
    assert_eq!(c.post("/api/open", json!({"relpath": "img\\chart.png", "mode": "reveal"})).await.status, 200);
    assert_eq!(c.post("/api/open", json!({"url": "https://example.org"})).await.status, 200);
    assert_eq!(c.post("/api/open", json!({"url": "file:///c:/windows"})).await.status, 400);
    assert_eq!(c.post("/api/open", json!({"relpath": "..\\outside.txt"})).await.status, 400);
    assert_eq!(c.post("/api/open", json!({"relpath": "img\\nothere.png"})).await.status, 404);
    let modes: Vec<String> = opened.lock().unwrap().iter().map(|(m, _)| m.clone()).collect();
    assert_eq!(modes, vec!["default", "reveal", "default"]);

    let md = r["reading"]["primary_file_id"].as_str().unwrap().to_string();
    let res = c.get(&format!("/api/resolve?href=../lib/smith_1776_wealth.pdf&from={}", md)).await.json;
    assert_eq!(res["ref_id"], json!(smith));
    let res = c.get(&format!("/api/resolve?href=img/chart.png%23top&from={}", md)).await.json;
    assert_eq!(res, json!({"file_id": fid, "ref_id": chart}));
    assert_eq!(c.get(&format!("/api/resolve?href=https://example.org/page&from={}", md)).await.json, json!({"external": true}));
    let res = c.get(&format!("/api/resolve?href=../notes/skip.txt&from={}", md)).await.json;
    assert_eq!(res["external"], json!(true));
    assert_eq!(res["relpath"], json!("notes\\skip.txt"));

    let mcp = c.get("/api/mcp").await.json;
    let cmd = mcp["command"].as_str().unwrap();
    assert!(cmd.starts_with("claude mcp add kurasi -s user -- \""), "{}", cmd);
    assert!(cmd.ends_with(&format!(" mcp --project \"{}\" --mode write", proj.to_string_lossy())), "{}", cmd);
    assert_eq!(mcp["tools"].as_array().unwrap().len(), 14);
    let modes: std::collections::BTreeSet<&str> = mcp["tools"].as_array().unwrap().iter().map(|t| t["mode"].as_str().unwrap()).collect();
    assert_eq!(modes.into_iter().collect::<Vec<_>>(), vec!["propose", "read", "write"]);
    assert!(mcp["resources"].as_array().unwrap().iter().any(|x| x["uri"] == json!("kurasi://tags")));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn project_open_and_switch() {
    let h = home();
    let ui = ui_dir(h.dir.path());
    let proj = folder(h.dir.path());
    let (app, _ws, _) = test_app(&h.paths, &ui);
    let c = Client::new(app);
    let first = c.open_project(&proj).await["project"]["id"].as_str().unwrap().to_string();
    let other = h.dir.path().join("other");
    make_pdf(&other.join("doc_2000_x.pdf"), None, "", "", &[]);
    let st = c.post("/api/project", json!({"folder": other.to_string_lossy()})).await;
    assert_eq!(st.status, 200, "{}", st.err());
    c.wait_scan().await;
    assert_eq!(st.json["project"]["folder"], json!(other.to_string_lossy()));
    let st = c.post("/api/project/open", json!({"id": first})).await;
    assert_eq!(st.status, 200, "{}", st.err());
    assert_eq!(st.json["project"]["id"], json!(first));
    assert_eq!(c.post("/api/project/open", json!({"id": "p-nothere00000"})).await.status, 404);
    assert_eq!(c.post("/api/project/open", json!({})).await.status, 400);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn demo_build_open_and_reset() {
    let h = home();
    let ui = ui_dir(h.dir.path());
    let proj = folder(h.dir.path());
    let (app, _ws, _) = test_app(&h.paths, &ui);
    let c = Client::new(app);
    let first = c.open_project(&proj).await["project"]["id"].as_str().unwrap().to_string();
    let st = c.post("/api/demo", json!({})).await;
    assert_eq!(st.status, 200, "{}", st.err());
    assert_eq!(st.json["demo"], json!(true));
    assert_eq!(st.json["previous_project_id"], json!(first));
    assert!(st.json["counts"]["references"].as_i64().unwrap() >= 10);
    let demo_folder = h.paths.demo_dir();
    assert_eq!(c.post("/api/project", json!({"folder": demo_folder.to_string_lossy()})).await.status, 400);
    let st = c.post("/api/demo", json!({"reset": true})).await;
    assert_eq!(st.status, 200, "{}", st.err());
    assert_eq!(st.json["previous_project_id"], json!(first));
    let st = c.post("/api/project/open", json!({"id": first})).await;
    assert_eq!(st.json["demo"], json!(false));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn document_notes_on_every_file_type() {
    let h = home();
    let ui = ui_dir(h.dir.path());
    let proj = folder(h.dir.path());
    let (app, _ws, _) = test_app(&h.paths, &ui);
    let c = Client::new(app);
    c.open_project(&proj).await;
    let r = c.refs().await;
    let pdf = r["smith_1776_wealth"]["primary_file_id"].as_str().unwrap().to_string();
    let md = r["reading"]["primary_file_id"].as_str().unwrap().to_string();
    let img = r["chart"]["primary_file_id"].as_str().unwrap().to_string();
    for fid in [&pdf, &md, &img] {
        let hl = c
            .post(
                "/api/highlights",
                json!({"file_id": fid, "page": null, "kind": "note", "quads": [], "verbatim": "", "analysis": "(P) Note on the file.",
                       "label": "", "tags": ["to-read"]}),
            )
            .await;
        assert_eq!(hl.status, 200, "{}", hl.err());
        assert_eq!(hl.json["kind"], json!("note"));
        assert_eq!(hl.json["label"], json!("P"));
        assert_eq!(hl.json["tags"], json!(["to-read"]));
    }
    let notes = |v: Value| -> Vec<Value> { v.as_array().unwrap().iter().filter(|x| x["kind"] == json!("note")).cloned().collect() };
    let pdf_note = notes(c.get(&format!("/api/highlights?file_id={}", pdf)).await.json);
    assert_eq!(pdf_note[0]["page"], json!(1));
    // The PDF note is a real annotation: a full rescan keeps exactly one copy of it.
    c.post("/api/scan", json!({"full": true})).await;
    c.wait_scan().await;
    let again = notes(c.get(&format!("/api/highlights?file_id={}", pdf)).await.json);
    assert_eq!(again.iter().map(|x| x["id"].clone()).collect::<Vec<_>>(), vec![pdf_note[0]["id"].clone()]);
    assert_eq!(c.post("/api/highlights", json!({"file_id": md, "kind": "region"})).await.status, 400);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn switching_projects_under_load_never_fails() {
    let h = home();
    let ui = ui_dir(h.dir.path());
    let (app, _ws, _) = test_app(&h.paths, &ui);
    let c = Client::new(app);
    let mut ids = Vec::new();
    for name in ["one", "two"] {
        let dir = h.dir.path().join(name);
        make_pdf(&dir.join(format!("{}_2000_doc.pdf", name)), None, "", "", &[]);
        let st = c.post("/api/project", json!({"folder": dir.to_string_lossy()})).await;
        assert_eq!(st.status, 200, "{}", st.err());
        ids.push(st.json["project"]["id"].as_str().unwrap().to_string());
        c.wait_scan().await;
    }
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mut readers = Vec::new();
    for _ in 0..4 {
        let c = c.clone();
        let stop = stop.clone();
        readers.push(tokio::spawn(async move {
            let mut statuses = Vec::new();
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                for path in ["/api/state", "/api/references", "/api/tags", "/api/changes?since=0"] {
                    statuses.push(c.get(path).await.status);
                }
            }
            statuses
        }));
    }
    let mut switchers = Vec::new();
    for _ in 0..2 {
        let c = c.clone();
        let ids = ids.clone();
        switchers.push(tokio::spawn(async move {
            let mut statuses = Vec::new();
            for i in 0..8 {
                statuses.push(c.post("/api/project/open", json!({"id": ids[i % 2]})).await.status);
            }
            statuses
        }));
    }
    let mut all = Vec::new();
    for s in switchers {
        all.extend(s.await.unwrap());
    }
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    for r in readers {
        all.extend(r.await.unwrap());
    }
    assert!(!all.is_empty());
    let bad: Vec<u16> = all.into_iter().filter(|s| *s != 200).collect();
    assert!(bad.is_empty(), "statuses other than 200: {:?}", bad);
}

// ------------------------------------------------------------------ catalogue payload, search, compression

const LIST_KEYS: [&str; 21] = [
    "id", "title", "authors", "kind", "pub", "year", "date", "lang", "status", "register_id", "categories", "ext", "pages",
    "hl_count", "top_tags", "missing", "files", "primary_file_id", "thumb_url", "created_at", "updated_at",
];

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn catalogue_list_without_bulk_text_and_ids_filter() {
    let h = home();
    let ui = ui_dir(h.dir.path());
    let proj = folder(h.dir.path());
    let (app, _ws, _) = test_app(&h.paths, &ui);
    let c = Client::new(app);
    c.open_project(&proj).await;
    let list = c.list().await;
    assert_eq!(list.len(), 7);
    for r in &list {
        assert_eq!(keys(r), LIST_KEYS.iter().map(|s| s.to_string()).collect::<Vec<_>>(), "list record {}", r["id"]);
        for f in r["files"].as_array().unwrap() {
            assert_eq!(keys(f), vec!["id", "relpath", "role", "ext", "pages", "missing", "cloud_only"]);
        }
    }
    // The full record keeps everything, and the list fields equal it.
    let full = c.refs().await;
    let smith = &full["smith_1776_wealth"];
    assert!(smith["search_text"].as_str().unwrap().contains("division of labour"));
    assert!(has_keys(smith, &["slug", "usability", "tags", "search_text", "doi", "url", "vol", "issue", "pages_range"]));
    let sid = smith["id"].as_str().unwrap().to_string();
    let row = list.iter().find(|r| r["id"] == json!(sid)).unwrap().clone();
    for k in LIST_KEYS {
        if k != "files" {
            assert_eq!(row[k], smith[k], "field {}", k);
        }
    }
    assert_eq!(row["files"].as_array().unwrap().len(), smith["files"].as_array().unwrap().len());

    // ?ids= returns the named references only; unknown ids are left out.
    let note = full["reading"]["id"].as_str().unwrap().to_string();
    let r = c.get(&format!("/api/references?ids={},r-000000000000,{}", sid, note)).await;
    assert_eq!(r.status, 200);
    let mut got: Vec<String> = r.json.as_array().unwrap().iter().map(|x| x["id"].as_str().unwrap().to_string()).collect();
    got.sort();
    let mut want = vec![sid.clone(), note.clone()];
    want.sort();
    assert_eq!(got, want);
    assert_eq!(r.json.as_array().unwrap().iter().find(|x| x["id"] == json!(sid)).unwrap(), &row);
    assert_eq!(c.get("/api/references?ids=").await.json, json!([]));
    assert_eq!(c.get(&format!("/api/references?ids=%20{}%20", note)).await.json.as_array().unwrap().len(), 1);
    // After an edit the partial list carries the new value; a reference in the trash is left out.
    c.put(&format!("/api/reference/{}", sid), json!({"title": "The wealth of nations, edited"})).await;
    let r = c.get(&format!("/api/references?ids={}", sid)).await;
    assert_eq!(r.json[0]["title"], json!("The wealth of nations, edited"));
    let tid = c.delete(&format!("/api/reference/{}", sid)).await.json["trash_id"].as_i64().unwrap();
    assert_eq!(c.get(&format!("/api/references?ids={}", sid)).await.json, json!([]));
    c.post("/api/undo", json!({"trash_id": tid})).await;
    assert_eq!(c.get(&format!("/api/references?ids={}", sid)).await.json.as_array().unwrap().len(), 1);
}

async fn search_json(c: &Client, q: &str, loose: Option<bool>) -> Value {
    let url = match loose {
        Some(l) => format!("/api/search?q={}&loose={}", q, if l { 1 } else { 0 }),
        None => format!("/api/search?q={}", q),
    };
    let r = c.get(&url).await;
    assert_eq!(r.status, 200, "{}", r.err());
    r.json
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn search_route_scores_full_text_per_token() {
    use kurasi_core::search::{ui_field_sim, LOOSE, STRICT};
    let h = home();
    let ui = ui_dir(h.dir.path());
    let proj = folder(h.dir.path());
    let (app, _ws, _) = test_app(&h.paths, &ui);
    let c = Client::new(app);
    c.open_project(&proj).await;
    let full = c.refs().await;
    let sid = full["smith_1776_wealth"]["id"].as_str().unwrap().to_string();
    // Substring hit, case folded; the tokens come back lower-cased.
    let r = search_json(&c, "LABOUR", Some(true)).await;
    assert_eq!(r["tokens"], json!(["labour"]));
    assert_eq!(r["loose"], json!(true));
    assert_eq!(r["hits"], json!([{"id": sid, "score": 1.0, "scores": [1.0]}]));
    // A typo: dice("labor", "labour") = 6/9, kept loose (0.42) and dropped strict (0.72).
    let r = search_json(&c, "labor", Some(true)).await;
    let s = r["hits"][0]["scores"][0].as_f64().unwrap();
    assert!((s - 0.9 * 6.0 / 9.0).abs() < 1e-12, "{}", s);
    assert_eq!(search_json(&c, "labor", Some(false)).await["hits"], json!([]));
    // dice("interpretaton", "interpretation") = 22/25 passes strict too.
    assert_eq!(search_json(&c, "interpretaton", Some(false)).await["hits"][0]["id"], json!(sid));
    // No match, and a query of whitespace only.
    assert_eq!(search_json(&c, "xylophone", Some(true)).await["hits"], json!([]));
    let r = search_json(&c, "%20%C2%A0", Some(true)).await;
    assert_eq!((r["tokens"].clone(), r["hits"].clone()), (json!([]), json!([])));
    // Per-token scores: a token the text misses scores 0 and the reference still lists, so the
    // UI can take that token from the metadata (here "wealth" from the title).
    let r = search_json(&c, "labour%20wealth", Some(true)).await;
    assert_eq!(r["tokens"], json!(["labour", "wealth"]));
    assert_eq!(r["hits"][0]["scores"], json!([1.0, 0.0]));
    assert_eq!(r["hits"][0]["score"], json!(1.0));
    // JavaScript whitespace splits tokens, NBSP and U+FEFF included.
    assert_eq!(search_json(&c, "labour%C2%A0output%EF%BB%BFsecond", Some(true)).await["tokens"], json!(["labour", "output", "second"]));
    // Every score equals the UI rule applied to the full record's search_text.
    for q in ["division", "labor", "interpretaton", "proposl", "raises", "output", "ofthe", "paragraf"] {
        for loose in [true, false] {
            let r = search_json(&c, q, Some(loose)).await;
            let thr = if loose { LOOSE } else { STRICT };
            for rec in full.values() {
                let want = ui_field_sim(q, rec["search_text"].as_str().unwrap(), thr);
                let got = r["hits"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|x| x["id"] == rec["id"])
                    .map(|x| x["scores"][0].as_f64().unwrap())
                    .unwrap_or(0.0);
                assert_eq!(got, want, "query {} loose {} ref {}", q, loose, rec["slug"]);
            }
        }
    }
    // Without the loose parameter the project setting applies.
    assert_eq!(search_json(&c, "labor", None).await["loose"], json!(true));
    c.put("/api/settings", json!({"loose": false})).await;
    let r = search_json(&c, "labor", None).await;
    assert_eq!((r["loose"].clone(), r["hits"].clone()), (json!(false), json!([])));
    // The index follows the change feed: an edited analysis is found, a reference in the trash is not.
    let hl = c.get(&format!("/api/highlights?ref_id={}", sid)).await.json[0].clone();
    let up = c.put(&format!("/api/highlights/{}", hl["id"].as_str().unwrap()), json!({"analysis": "(F) Zymurgy tested."})).await;
    assert_eq!(up.status, 200, "{}", up.err());
    assert_eq!(search_json(&c, "zymurgy", Some(true)).await["hits"][0]["id"], json!(sid));
    let tid = c.delete(&format!("/api/reference/{}", sid)).await.json["trash_id"].as_i64().unwrap();
    assert_eq!(search_json(&c, "zymurgy", Some(true)).await["hits"], json!([]));
    c.post("/api/undo", json!({"trash_id": tid})).await;
    assert_eq!(search_json(&c, "zymurgy", Some(true)).await["hits"][0]["id"], json!(sid));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn compression_for_text_bodies_only() {
    let h = home();
    let ui = ui_dir(h.dir.path());
    std::fs::write(ui.join("app.js"), "/* ui */\n".repeat(400)).unwrap();
    let proj = folder(h.dir.path());
    let (app, _ws, _) = test_app(&h.paths, &ui);
    let plain = Client::new(app);
    plain.open_project(&proj).await;
    let gz = plain.with_accept_encoding("gzip, deflate, br, zstd");
    let a = plain.get("/api/references").await;
    let b = gz.get("/api/references").await;
    assert_eq!((a.encoding.as_str(), b.encoding.as_str()), ("", "gzip"));
    assert!(b.bytes.len() * 2 < a.bytes.len(), "{} of {} bytes", b.bytes.len(), a.bytes.len());
    assert_eq!(a.json, b.json);
    assert_eq!(b.ctype, "application/json");
    let js = gz.get("/ui/app.js").await;
    assert_eq!((js.status, js.encoding.as_str()), (200, "gzip"));
    // Small bodies, page images, thumbnails and PDF bytes are sent as they are.
    assert_eq!(gz.get("/api/changes?since=999999&timeout=0").await.encoding, "");
    let refs = plain.refs().await;
    let pdf = refs["smith_1776_wealth"]["primary_file_id"].as_str().unwrap().to_string();
    let png = gz.get(&format!("/api/file/{}/page/1.png", pdf)).await;
    assert_eq!((png.status, png.encoding.as_str(), &png.bytes[..4]), (200, "", &b"\x89PNG"[..]));
    let thumb = gz.get(&format!("/api/thumb/{}.png", pdf)).await;
    assert_eq!((thumb.status, thumb.encoding.as_str()), (200, ""));
    let raw = gz.get(&format!("/api/file/{}/raw", pdf)).await;
    assert_eq!((raw.encoding.as_str(), &raw.bytes[..4]), ("", &b"%PDF"[..]));
    // A search answer and an error stay readable JSON through the layer.
    let s = gz.get("/api/search?q=a").await;
    assert_eq!((s.status, s.json["tokens"].clone()), (200, json!(["a"])));
    let e = gz.get("/api/nope").await;
    assert_eq!((e.status, e.json["error"].is_string()), (404, true));
}

/// The project list, the folder check, removal with and without data (the open project and a
/// pending long-poll included), restore, and the sample project's Reset.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn projects_list_check_remove_restore_and_reset() {
    let h = home();
    let ui = ui_dir(h.dir.path());
    let proj = folder(h.dir.path());
    let (app, _ws, _) = test_app(&h.paths, &ui);
    let c = Client::new(app);
    let a = c.open_project(&proj).await["project"]["id"].as_str().unwrap().to_string();
    let other = h.dir.path().join("other");
    make_pdf(&other.join("doc_2000_x.pdf"), None, "", "", &[]);
    std::fs::write(other.join("draft.txt"), "a file of the user").unwrap();
    let st = c.post("/api/project", json!({"folder": other.to_string_lossy()})).await;
    assert_eq!(st.status, 200, "{}", st.err());
    c.wait_scan().await;
    let b = st.json["project"]["id"].as_str().unwrap().to_string();
    assert_eq!(c.put("/api/settings", json!({"name": "Other"})).await.status, 200);

    // The list: last opened first, the open project marked.
    let list = c.get("/api/projects").await.json;
    let ids: Vec<&str> = list.as_array().unwrap().iter().map(|p| p["id"].as_str().unwrap()).collect();
    assert_eq!(ids, vec![b.as_str(), a.as_str()]);
    assert_eq!(list[0]["open"], json!(true));
    assert_eq!(list[1]["open"], json!(false));
    assert!(has_keys(&list[0], &["id", "name", "folder", "created", "last_opened", "demo", "exists", "open"]));
    assert_eq!(c.get("/api/state").await.json["projects"][0]["open"], json!(true));

    // The folder check.
    let ok = c.post("/api/project/check", json!({"folder": proj.to_string_lossy()})).await.json;
    assert_eq!(ok["ok"], json!(true));
    assert_eq!(ok["name"], json!("proj"));
    assert_eq!(ok["project_id"], json!(a));
    let fresh = h.dir.path().join("fresh");
    std::fs::create_dir_all(&fresh).unwrap();
    let ok = c.post("/api/project/check", json!({"folder": format!("\"{}\"", fresh.to_string_lossy())})).await.json;
    assert_eq!((ok["ok"].clone(), ok["name"].clone(), ok.get("project_id").is_none()), (json!(true), json!("fresh"), true));
    for (folder, says) in [
        (String::new(), "Enter the folder"),
        ("Papers\\Thesis".to_string(), "full path"),
        (fresh.join("nothing").to_string_lossy().into_owned(), "does not exist"),
        (other.join("draft.txt").to_string_lossy().into_owned(), "is a file"),
        (h.paths.demo_dir().to_string_lossy().into_owned(), "does not exist"),
    ] {
        let r = c.post("/api/project/check", json!({"folder": folder})).await.json;
        assert_eq!(r["ok"], json!(false), "{}", folder);
        assert!(r["error"].as_str().unwrap().contains(says), "{}: {}", folder, r["error"]);
    }

    // The data a removal would delete, then a removal with data of a closed project.
    let entry = c.get(&format!("/api/projects/{}", a)).await;
    assert_eq!(entry.status, 200, "{}", entry.err());
    let app_dir = h.paths.root.join("projects").join(&a);
    assert_eq!(entry.json["data"]["app"]["path"], json!(app_dir.to_string_lossy()));
    assert_eq!(entry.json["data"]["app"]["whole"], json!(true));
    assert_eq!(c.get("/api/projects/p-nothere00000").await.status, 404);
    let pdf = std::fs::read(proj.join("lib/smith_1776_wealth.pdf")).unwrap();
    let r = c.delete(&format!("/api/projects/{}?delete_data=1", a)).await;
    assert_eq!(r.status, 200, "{}", r.err());
    assert_eq!(r.json["closed"], json!(false));
    assert_eq!(r.json["with_data"], json!(true));
    let rid = r.json["removal_id"].as_str().unwrap().to_string();
    assert!(!app_dir.exists());
    assert!(!proj.join(".kurasi").join("metadata.json").exists());
    assert_eq!(std::fs::read(proj.join("lib/smith_1776_wealth.pdf")).unwrap(), pdf);
    assert!(proj.join("notes/reading.md").is_file() && proj.join("img/chart.png").is_file());
    assert_eq!(c.get("/api/projects").await.json.as_array().unwrap().len(), 1);
    let r = c.post("/api/projects/restore", json!({"removal_id": rid})).await;
    assert_eq!(r.status, 200, "{}", r.err());
    assert_eq!(r.json["project"]["id"], json!(a));
    assert!(app_dir.join("kurasi.sqlite").is_file());
    assert_eq!(c.post("/api/projects/restore", json!({"removal_id": rid})).await.status, 404);
    assert_eq!(c.post("/api/projects/restore", json!({})).await.status, 400);
    assert_eq!(c.delete("/api/projects/p-nothere00000").await.status, 404);

    // The open project, with data, while a long-poll of /api/changes holds it.
    let seq = c.get("/api/state").await.json["change_seq"].as_i64().unwrap();
    let poll = {
        let c = c.clone();
        tokio::spawn(async move { c.get(&format!("/api/changes?since={}&timeout=30", seq)).await })
    };
    tokio::time::sleep(Duration::from_millis(300)).await;
    let t0 = Instant::now();
    let r = c.delete(&format!("/api/projects/{}?delete_data=1", b)).await;
    assert_eq!(r.status, 200, "{}", r.err());
    // The pending long-poll lets go of the project at once; its own timeout is 30 s. The bound
    // leaves room for a loaded test machine, where closing the watcher and the runner is slow.
    eprintln!("removal of the open project took {:?}", t0.elapsed());
    assert!(t0.elapsed() < Duration::from_secs(25), "removal took {:?}", t0.elapsed());
    assert_eq!(r.json["closed"], json!(true));
    let polled = tokio::time::timeout(Duration::from_secs(5), poll).await.expect("the long-poll answers").unwrap();
    assert_eq!(polled.status, 200);
    let st = c.get("/api/state").await.json;
    assert!(st["project"].is_null());
    assert_eq!(st["projects"].as_array().unwrap().len(), 1);
    assert!(other.join("draft.txt").is_file() && other.join("doc_2000_x.pdf").is_file());
    assert!(!h.paths.root.join("projects").join(&b).exists());
    let rid = r.json["removal_id"].as_str().unwrap().to_string();
    let r = c.post("/api/projects/restore", json!({"removal_id": rid})).await;
    assert_eq!(r.status, 200, "{}", r.err());
    assert!(c.get("/api/state").await.json["project"].is_null(), "a restored project stays closed");
    let st = c.post("/api/project/open", json!({"id": b})).await;
    assert_eq!(st.status, 200, "{}", st.err());
    assert_eq!(st.json["project"]["name"], json!("Other"));
    assert_eq!(st.json["counts"]["references"], json!(1));

    // Without data: the entry goes, the database stays.
    let r = c.delete(&format!("/api/projects/{}", a)).await;
    assert_eq!(r.status, 200, "{}", r.err());
    assert!(app_dir.join("kurasi.sqlite").is_file());
    assert_eq!(r.json["moved"], json!([]));

    // The sample project: Reset, never Remove.
    assert_eq!(c.post("/api/demo", json!({})).await.status, 200);
    let r = c.delete("/api/projects/demo?delete_data=1").await;
    assert_eq!(r.status, 400);
    assert!(r.err().contains("Reset"), "{}", r.err());
    let r = c.post("/api/projects/demo/reset", json!({})).await;
    assert_eq!(r.status, 200, "{}", r.err());
    assert_eq!(r.json["open"], json!(true));
    let st = c.get("/api/state").await.json;
    assert_eq!(st["demo"], json!(true));
    assert_eq!(st["previous_project_id"], json!(b));
    assert_eq!(c.post(&format!("/api/projects/{}/reset", b), json!({})).await.status, 400);
    assert!(c.get("/api/projects").await.json.as_array().unwrap().iter().any(|p| p["demo"] == json!(true)));
}
