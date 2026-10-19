//! Every MCP tool and resource called in-process (a port of tests/test_mcp.py), the three
//! modes, and stdio round trips against the sample project through the test binary (the
//! initialize handshake and the 2026-07-28 server/discover handshake).

mod common;

use std::collections::BTreeSet;
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use common::{folder, home, ref_by_slug, scanned};
use kurasi_core::model::{HighlightFilter, Origin};
use kurasi_server::{Mode, Tools, TOOLS};
use serde_json::{json, Map, Value};

fn args(v: Value) -> Map<String, Value> {
    v.as_object().cloned().unwrap_or_default()
}

fn data(r: Result<Value, String>) -> Value {
    r.expect("tool succeeds")
}

fn error(r: Result<Value, String>) -> String {
    match r {
        Ok(v) => panic!("expected a tool error, got {}", v),
        Err(e) => e,
    }
}

#[test]
fn every_tool_and_resource() {
    let h = home();
    let proj = folder(h.dir.path());
    let p = scanned(&h.paths, &proj);
    let tools = Tools::new(p.clone());
    let smith = ref_by_slug(&p, "smith_1776_wealth");
    let chart = ref_by_slug(&p, "chart");
    let note = ref_by_slug(&p, "reading");
    let news = ref_by_slug(&p, "2026-03-02_news-item");
    let hl = p.highlights(&HighlightFilter { ref_id: Some(smith.id.clone()), ..Default::default() }).unwrap().remove(0);
    let seq0 = p.change_seq();
    let client = "pytest-agent";
    let call = |tool: &str, a: Value| tools.run(tool, args(a), client, Mode::Write);

    let listed: BTreeSet<String> = kurasi_server::mcp::tool_definitions().iter().map(|t| t.name.to_string()).collect();
    assert_eq!(listed, TOOLS.iter().map(|t| t.0.to_string()).collect::<BTreeSet<_>>());
    assert_eq!(listed.len(), 14);

    let search = data(call("catalog_search", json!({"query": "welth nations", "limit": 5})));
    assert_eq!(search["results"][0]["id"], json!(smith.id));
    let get = data(call("reference_get", json!({"ref_id": smith.id})));
    assert_eq!(get["id"], json!(smith.id));
    assert!(get.get("search_text").is_none());
    assert_eq!(get["hl_count"], json!(3));
    let text = data(call("reference_text", json!({"ref_id": smith.id, "page": 1})));
    assert_eq!(text["words"][0][4], json!("The"));
    assert!(text["text"].as_str().unwrap().contains("division of labour"));
    assert_eq!(text["pages"], json!(3));
    let md_text = data(call("reference_text", json!({"ref_id": note.id})));
    assert!(md_text["text"].as_str().unwrap().starts_with("# Notes"));
    assert!(error(call("reference_text", json!({"ref_id": chart.id}))).contains("no text layer"));
    let hls = data(call("highlights_list", json!({"ref_id": smith.id})));
    assert_eq!(hls["highlights"].as_array().unwrap().len(), 3);
    let hls_tag = data(call("highlights_list", json!({"tag": "#quote"})));
    assert_eq!(hls_tag["highlights"].as_array().unwrap().iter().map(|x| x["tags"].clone()).collect::<Vec<_>>(), vec![json!(["division-of-labour", "quote"])]);
    let hls_author = data(call("highlights_list", json!({"author": "claude"})));
    let authors: BTreeSet<String> = hls_author["highlights"].as_array().unwrap().iter().map(|x| x["author"].as_str().unwrap().to_string()).collect();
    assert_eq!(authors.into_iter().collect::<Vec<_>>(), vec!["Claude"]);
    let tags = data(call("tags_list", json!({})));
    let names: BTreeSet<&str> = tags["tags"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert!(names.contains("quote") && names.contains("efficiency"));
    let cats = data(call("categories_list", json!({})));
    assert_eq!(cats["categories"][0]["name"], json!("Uncategorized"));
    let meta = data(call(
        "reference_update_metadata",
        json!({"ref_id": news.id, "fields": {"title": "Wage talks stall", "kind": "News article", "pub": "Example News", "status": "Verified"},
               "authors": ["Example News"], "categories": ["Press/English language"], "purpose": "Fill metadata from the first page"}),
    ));
    assert_eq!(meta["status"], json!("Suggested"));
    assert_eq!(meta["title"], json!("Wage talks stall"));
    assert_eq!(meta["categories"], json!(["Press/English language"]));
    let catz = data(call("reference_categorize", json!({"ref_ids": [chart.id], "category_paths": ["Exhibits/Charts"], "purpose": "File the chart"})));
    assert_eq!(catz["references"][0]["categories"], json!(["Exhibits/Charts"]));
    assert_eq!(catz["references"][0]["status"], json!("Suggested"));
    let propose = data(call("highlight_propose_delete", json!({"highlight_id": hl.id, "reason": "Duplicate of page 1."})));
    assert_eq!(propose["suggestion"]["action"], json!("delete"));
    assert!(propose["suggestion"]["resolution"].is_null());
    let suggest = data(call("highlight_suggest_tags", json!({"highlight_id": hl.id, "tags": ["method", "#to-read"]})));
    assert_eq!(suggest["suggested_tags"], json!(["method", "to-read"]));
    let add = data(call(
        "highlight_add",
        json!({"ref_id": smith.id, "analysis": "(I) Pay follows output.", "label": "I", "tags": ["incentives"],
               "quote": "measured  tasks and a bonus", "purpose": "Mark the bonus passage"}),
    ));
    assert_eq!(
        (add["author"].clone(), add["origin"].clone(), add["verbatim"].clone(), add["page"].clone(), add["label"].clone()),
        (json!("Claude"), json!("ai"), json!("Measured tasks and a bonus"), json!(3), json!("I"))
    );
    let add_quads = data(call("highlight_add", json!({"ref_id": smith.id, "analysis": "(F) Box.", "page": 2, "quads": [[72, 72, 200, 90]]})));
    assert_eq!(add_quads["page"], json!(2));
    let missing = error(call("highlight_add", json!({"ref_id": smith.id, "analysis": "x", "quote": "words that are nowhere", "page": 1})));
    assert!(missing.contains("quote was not found on page 1"), "{}", missing);
    let add_md = data(call("highlight_add", json!({"ref_id": note.id, "analysis": "(F) The note says so.", "tags": ["quote"], "quote": "the smith copy"})));
    assert_eq!(add_md["anchor"], json!({"text": "the Smith copy", "occurrence": 0}));
    let add_img = data(call("highlight_add", json!({"ref_id": chart.id, "analysis": "(F) Bars.", "quads": [[0.1, 0.1, 0.5, 0.5]], "quote": "Bars"})));
    assert_eq!(add_img["kind"], json!("region"));
    assert_eq!(add_img["verbatim"], json!("Bars"));
    let scan = data(call("catalog_scan", json!({"full": false})));
    assert_eq!(scan["changed"], json!(0));
    let export = data(call("export_references", json!({"format": "bibtex"})));
    assert_eq!(export["path"], json!(proj.join("_exports").join("references.bib").to_string_lossy()));
    assert!(error(call("export_references", json!({"format": "docx"}))).contains("format must be one of"));
    // A quiet write (no catalogue row changed) still adds a change row and fires the broadcast.
    let mut rx = p.subscribe();
    let seq_quiet = p.change_seq();
    let log = data(call("log_append", json!({"purpose": "Checked every quote", "checks": "Compared each quote with its page"})));
    assert_eq!(log, json!({"ok": true}));
    assert_eq!(p.change_seq(), seq_quiet + 1);
    assert_eq!(rx.try_recv().unwrap(), seq_quiet + 1);
    assert!(error(call("reference_get", json!({"ref_id": "r-000000000000"}))).contains("does not exist"));

    let r_refs: Value = serde_json::from_str(&tools.read_resource("kurasi://references").unwrap()).unwrap();
    assert_eq!(r_refs.as_array().unwrap().len(), 7);
    assert!(r_refs[0].get("search_text").is_none());
    let r_ref: Value = serde_json::from_str(&tools.read_resource(&format!("kurasi://reference/{}", smith.id)).unwrap()).unwrap();
    assert_eq!(r_ref["id"], json!(smith.id));
    let r_hls: Value = serde_json::from_str(&tools.read_resource(&format!("kurasi://reference/{}/highlights", smith.id)).unwrap()).unwrap();
    assert_eq!(r_hls.as_array().unwrap().len(), 5);
    let r_tags: Value = serde_json::from_str(&tools.read_resource("kurasi://tags").unwrap()).unwrap();
    assert!(!r_tags.as_array().unwrap().is_empty());
    assert!(tools.read_resource("kurasi://nothing").is_err());
    assert!(tools.read_resource("kurasi://reference/r-000000000000").is_err());

    // Every tool call left an ai_log row with the client name; writes left change rows with origin mcp.
    let rows = p.ai_log().unwrap();
    assert_eq!(rows.len(), 24);
    assert!(rows.iter().all(|r| r.client == "pytest-agent"));
    let meta_row = rows.iter().find(|r| r.tool == "reference_update_metadata").unwrap();
    assert_eq!(meta_row.purpose, "Fill metadata from the first page");
    assert_eq!(meta_row.args["previous"]["title"], json!("2026-03-02_news-item"));
    assert_eq!(meta_row.args["previous"]["status"], json!("Auto"));
    let errors = rows.iter().filter(|r| r.result.starts_with("Error")).count();
    assert_eq!(errors, 4);
    let log_row = rows.iter().find(|r| r.tool == "log_append").unwrap();
    assert_eq!(log_row.args["checks"], json!("Compared each quote with its page"));
    assert_eq!(rows[0].tool, "reference_get");
    let changes = p.changes_since(seq0).unwrap().changes;
    assert!(!changes.is_empty());
    assert!(changes.iter().all(|c| c.origin == "mcp"), "{:?}", changes);
    // 8 catalogue writes plus the quiet scan, the export and the log entry.
    assert_eq!(changes.len(), 11, "{:?}", changes.iter().map(|c| &c.what).collect::<Vec<_>>());
}

#[test]
fn read_only_project_refuses_file_writes() {
    let h = home();
    let proj = folder(h.dir.path());
    let p = scanned(&h.paths, &proj);
    p.set_read_only(true).unwrap();
    let tools = Tools::new(p.clone());
    let smith = ref_by_slug(&p, "smith_1776_wealth");
    let add = tools.run("highlight_add", args(json!({"ref_id": smith.id, "analysis": "x", "quote": "division of labour"})), "t", Mode::Write);
    let exp = tools.run("export_references", args(json!({"format": "apa-md"})), "t", Mode::Write);
    let meta = tools.run("reference_update_metadata", args(json!({"ref_id": smith.id, "fields": {"year": "1776"}})), "t", Mode::Write);
    assert!(error(add).contains("read-only"));
    assert!(error(exp).contains("read-only"));
    assert_eq!(data(meta)["status"], json!("Suggested"));
}

#[test]
fn modes_refuse_tools_above_them() {
    let h = home();
    let proj = folder(h.dir.path());
    let p = scanned(&h.paths, &proj);
    let tools = Tools::new(p.clone());
    let smith = ref_by_slug(&p, "smith_1776_wealth");
    let rows_before = p.ai_log().unwrap().len();
    let seq_before = p.change_seq();
    assert!(data(tools.run("reference_get", args(json!({"ref_id": smith.id})), "t", Mode::Read))["id"] == json!(smith.id));
    let refused = error(tools.run("reference_categorize", args(json!({"ref_ids": [smith.id], "category_paths": ["X"]})), "t", Mode::Read));
    assert!(refused.contains("read mode") && refused.contains("propose tool"), "{}", refused);
    let refused = error(tools.run("log_append", args(json!({"purpose": "p", "checks": "c"})), "t", Mode::Read));
    assert!(refused.contains("write tool"), "{}", refused);
    let refused = error(tools.run("export_references", args(json!({"format": "bibtex"})), "t", Mode::Propose));
    assert!(refused.contains("propose mode"), "{}", refused);
    assert!(data(tools.run("highlight_suggest_tags", args(json!({"highlight_id": p.highlights(&HighlightFilter::default()).unwrap()[0].id, "tags": "a b"})), "t", Mode::Propose))["suggested_tags"] == json!(["a", "b"]));
    assert!(error(tools.run("no_such_tool", args(json!({})), "t", Mode::Write)).contains("Unknown tool"));
    let rows = p.ai_log().unwrap();
    assert_eq!(rows.len(), rows_before + 6);
    assert_eq!(rows.iter().filter(|r| r.result.starts_with("Error")).count(), 4);
    let changes = p.changes_since(seq_before).unwrap().changes;
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].origin, Origin::Mcp.as_str());
    assert!(Mode::Read < Mode::Propose && Mode::Propose < Mode::Write);
    assert_eq!("WRITE".parse::<Mode>(), Ok(Mode::Write));
    assert!("admin".parse::<Mode>().is_err());
}

// ------------------------------------------------------------------ stdio round trip

struct StdioClient {
    child: std::process::Child,
    stdin: std::process::ChildStdin,
    stdout: BufReader<std::process::ChildStdout>,
    next_id: i64,
}

impl StdioClient {
    fn spawn(home: &std::path::Path, project: &str, mode: &str) -> StdioClient {
        let exe = env!("CARGO_BIN_EXE_kurasi-mcp-test");
        let mut child = Command::new(exe)
            .arg(project)
            .arg(mode)
            .env("KURASI_HOME", home)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("test binary starts");
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        StdioClient { child, stdin, stdout, next_id: 0 }
    }

    fn notify(&mut self, method: &str, params: Value) {
        let msg = json!({"jsonrpc": "2.0", "method": method, "params": params});
        writeln!(self.stdin, "{}", msg).unwrap();
        self.stdin.flush().unwrap();
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        let msg = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        writeln!(self.stdin, "{}", msg).unwrap();
        self.stdin.flush().unwrap();
        loop {
            let mut line = String::new();
            let n = self.stdout.read_line(&mut line).expect("server output");
            assert!(n > 0, "the server closed stdout before answering {}", method);
            let v: Value = match serde_json::from_str(line.trim()) {
                Ok(v) => v,
                Err(_) => continue,
            };
            if v.get("id") == Some(&json!(id)) {
                if let Some(e) = v.get("error") {
                    panic!("{} failed: {}", method, e);
                }
                return v["result"].clone();
            }
        }
    }

    fn call(&mut self, tool: &str, arguments: Value) -> Value {
        self.request("tools/call", json!({"name": tool, "arguments": arguments}))
    }

    fn close(mut self) {
        drop(self.stdin);
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if let Ok(Some(_)) = self.child.try_wait() {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = self.child.kill();
    }
}

fn tool_json(result: &Value) -> Value {
    assert_ne!(result["isError"], json!(true), "tool error: {}", result);
    serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap()
}

#[test]
fn stdio_round_trip_on_the_sample_project() {
    let h = home();
    let entry = kurasi_core::demo::build(&h.paths).expect("sample project builds");
    let home_dir = h.paths.root.clone();

    let t0 = Instant::now();
    let mut c = StdioClient::spawn(&home_dir, &entry.id, "propose");
    let init = c.request(
        "initialize",
        json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "kurasi-rust-test", "version": "1"}}),
    );
    let cold_start = t0.elapsed();
    eprintln!("MCP cold start (spawn to initialize response, test build): {:?}", cold_start);
    assert_eq!(init["serverInfo"]["name"], json!("kurasi"));
    assert_eq!(init["serverInfo"]["version"], json!(env!("CARGO_PKG_VERSION")));
    assert!(init["capabilities"]["tools"].is_object() && init["capabilities"]["resources"].is_object());
    assert!(init["instructions"].as_str().unwrap().contains("propose"));
    assert!(cold_start < Duration::from_secs(5), "cold start took {:?}", cold_start);
    c.notify("notifications/initialized", json!({}));

    let tools = c.request("tools/list", json!({}));
    let names: BTreeSet<String> = tools["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap().to_string()).collect();
    assert_eq!(names.len(), 14);
    assert!(names.contains("catalog_search") && names.contains("log_append"));
    assert!(tools["tools"][0]["inputSchema"]["properties"].is_object());

    let search = tool_json(&c.call("catalog_search", json!({"query": "wealth of nations", "limit": 3})));
    assert!(!search["results"].as_array().unwrap().is_empty());
    let first = search["results"][0]["id"].as_str().unwrap().to_string();
    assert_eq!(c.call("catalog_search", json!({"query": "x"}))["structuredContent"]["query"], json!("x"));
    let got = tool_json(&c.call("reference_get", json!({"ref_id": first})));
    assert_eq!(got["id"], json!(first));
    assert_eq!(c.call("highlights_list", json!({"ref_id": "r-000000000000"}))["isError"], json!(false));

    // Propose mode: proposals pass, writes are refused with a clear message.
    let catz = tool_json(&c.call("reference_categorize", json!({"ref_ids": [first], "category_paths": ["Economics/Classics"]})));
    assert_eq!(catz["references"][0]["status"], json!("Suggested"));
    let refused = c.call("log_append", json!({"purpose": "p", "checks": "c"}));
    assert_eq!(refused["isError"], json!(true));
    assert!(refused["content"][0]["text"].as_str().unwrap().contains("propose mode"));

    let res = c.request("resources/list", json!({}));
    let uris: BTreeSet<String> = res["resources"].as_array().unwrap().iter().map(|r| r["uri"].as_str().unwrap().to_string()).collect();
    assert_eq!(uris, ["kurasi://references", "kurasi://tags"].iter().map(|s| s.to_string()).collect());
    let tpl = c.request("resources/templates/list", json!({}));
    let tpls: BTreeSet<String> = tpl["resourceTemplates"].as_array().unwrap().iter().map(|r| r["uriTemplate"].as_str().unwrap().to_string()).collect();
    assert_eq!(tpls, ["kurasi://reference/{ref_id}", "kurasi://reference/{ref_id}/highlights"].iter().map(|s| s.to_string()).collect());
    let tags = c.request("resources/read", json!({"uri": "kurasi://tags"}));
    let tag_list: Value = serde_json::from_str(tags["contents"][0]["text"].as_str().unwrap()).unwrap();
    assert!(!tag_list.as_array().unwrap().is_empty());
    assert_eq!(tags["contents"][0]["mimeType"], json!("application/json"));
    c.close();

    // The log holds every call with the client name from initialize.
    let project = kurasi_core::demo::open(&h.paths).unwrap();
    let rows = project.ai_log().unwrap();
    let mine: Vec<_> = rows.iter().filter(|r| r.client == "kurasi-rust-test").collect();
    assert_eq!(mine.len(), 6);
    assert!(mine.iter().any(|r| r.tool == "log_append" && r.result.starts_with("Error")));
}

/// The `_meta` of one request in the 2026-07-28 lifecycle. `client` None leaves out
/// `clientInfo`, which that protocol makes optional.
fn inline_meta(client: Option<&str>) -> Value {
    let mut meta = json!({
        "io.modelcontextprotocol/protocolVersion": "2026-07-28",
        "io.modelcontextprotocol/clientCapabilities": {},
    });
    if let Some(name) = client {
        meta["io.modelcontextprotocol/clientInfo"] = json!({"name": name, "version": "1"});
    }
    meta
}

#[test]
fn client_name_with_both_handshakes() {
    let h = home();
    let entry = kurasi_core::demo::build(&h.paths).expect("sample project builds");
    let home_dir = h.paths.root.clone();

    // The initialize handshake: the client name travels once, in `clientInfo`.
    let mut c = StdioClient::spawn(&home_dir, &entry.id, "read");
    c.request(
        "initialize",
        json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "legacy-agent", "version": "1"}}),
    );
    c.notify("notifications/initialized", json!({}));
    tool_json(&c.call("catalog_search", json!({"query": "wealth"})));
    c.close();

    // The 2026-07-28 handshake: server/discover, then the client name in the `_meta` of
    // every request. The last call leaves `clientInfo` out.
    let mut c = StdioClient::spawn(&home_dir, &entry.id, "read");
    let found = c.request("server/discover", json!({"_meta": inline_meta(Some("discover-agent"))}));
    assert!(found["supportedVersions"].as_array().unwrap().contains(&json!("2026-07-28")), "{}", found);
    let listed = c.request("tools/list", json!({"_meta": inline_meta(Some("discover-agent"))}));
    assert_eq!(listed["tools"].as_array().unwrap().len(), 14);
    let call = |tool: &str, arguments: Value, client: Option<&str>| json!({"name": tool, "arguments": arguments, "_meta": inline_meta(client)});
    tool_json(&c.request("tools/call", call("catalog_search", json!({"query": "wealth"}), Some("discover-agent"))));
    tool_json(&c.request("tools/call", call("tags_list", json!({}), None)));
    c.close();

    let project = kurasi_core::demo::open(&h.paths).unwrap();
    let rows = project.ai_log().unwrap();
    let pairs: BTreeSet<(String, String)> = rows.iter().map(|r| (r.tool.clone(), r.client.clone())).collect();
    let pair = |tool: &str, client: &str| (tool.to_string(), client.to_string());
    assert!(pairs.contains(&pair("catalog_search", "legacy-agent")), "{:?}", pairs);
    assert!(pairs.contains(&pair("catalog_search", "discover-agent")), "{:?}", pairs);
    assert!(pairs.contains(&pair("tags_list", "unknown client")), "{:?}", pairs);
    let named = rows.iter().filter(|r| r.client == "legacy-agent" || r.client == "discover-agent").count();
    assert_eq!(named, 2);
}
