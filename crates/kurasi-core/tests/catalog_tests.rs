mod common;

use common::*;
use kurasi_core::model::{HighlightFilter, HighlightPatch, NewHighlight, Origin, ReferenceFilter, ReferencePatch, Writer};
use kurasi_core::scanner::ScanMode;
use serde_json::{json, Map, Value};

fn fields(pairs: &[(&str, Value)]) -> Map<String, Value> {
    pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
}

#[test]
fn references_crud_undo_and_categories() {
    let h = home();
    let folder = folder(h.dir.path());
    let p = scanned(&h.paths, &folder);
    assert_eq!(p.counts().unwrap().references, 7, "_NOTES.md in two folders stays two references");
    let r = ref_by_slug(&p, "smith_1776_wealth");
    assert!(r.search_text.contains("division of labour"));
    assert!(r.doi.is_none(), "the list form carries no detail");
    let detail = p.reference(&r.id).unwrap();
    assert!(detail.doi.is_some() && detail.pages_range.is_some());
    let listed = p.references(&ReferenceFilter { ids: Some(vec![r.id.clone()]), detail: true, ..ReferenceFilter::default() }).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].detail("doi"), "");

    let patch = ReferencePatch {
        fields: fields(&[("title", json!("An inquiry")), ("kind", json!("Book")), ("pub", json!("Strahan")), ("doi", json!("10.1/x")), ("usability", json!("Full text"))]),
        authors: Some(vec!["Smith, A.".into()]),
        categories: Some(vec!["Economics/Classics".into(), "Notes".into()]),
    };
    let (upd, previous) = p.update_reference(&r.id, &patch, Origin::Ui, "Verified").unwrap();
    assert_eq!(upd.status, "Verified");
    assert_eq!(upd.doi.as_deref(), Some("10.1/x"));
    assert_eq!(upd.categories, vec!["Economics/Classics", "Notes"]);
    assert_eq!(previous["title"], json!("The wealth of nations"));
    let bad = p.update_reference(&r.id, &ReferencePatch { fields: fields(&[("usability", json!("Bad"))]), ..Default::default() }, Origin::Ui, "Verified").unwrap_err();
    assert_eq!(bad.status(), 400);
    let missing = p.update_reference("r-000000000000", &ReferencePatch { fields: fields(&[("title", json!("x"))]), ..Default::default() }, Origin::Ui, "Verified").unwrap_err();
    assert_eq!(missing.status(), 404);
    let (upd, _) = p.update_reference(&r.id, &ReferencePatch { fields: fields(&[("status", json!("Suggested"))]), ..Default::default() }, Origin::Ui, "Verified").unwrap();
    assert_eq!(upd.status, "Suggested");

    let cats = p.categories().unwrap();
    let econ = cats.iter().find(|c| c.name == "Economics").unwrap().clone();
    assert_eq!(econ.children[0].path, "Economics/Classics");
    let new = p.create_category(Some(econ.id), "Modern", Origin::Ui).unwrap();
    assert_eq!(new.path, "Economics/Modern");
    assert_eq!(p.create_category(Some(econ.id), "Modern", Origin::Ui).unwrap_err().status(), 409);
    assert_eq!(p.create_category(None, "a/b", Origin::Ui).unwrap_err().status(), 400);
    let moved = p.update_category(new.id, Some("Recent"), Some(None), Origin::Ui).unwrap();
    assert_eq!(moved.path, "Recent");
    let classics = econ.children[0].id;
    assert_eq!(p.update_category(econ.id, None, Some(Some(classics)), Origin::Ui).unwrap_err().status(), 400);
    assert_eq!(p.update_category(1, Some("x"), None, Origin::Ui).unwrap_err().status(), 400);
    assert_eq!(p.delete_category(classics, Origin::Ui).unwrap(), 1);
    let mut now = ref_by_slug(&p, "smith_1776_wealth").categories;
    now.sort();
    assert_eq!(now, vec!["Economics", "Notes"]);
    assert_eq!(p.delete_category(1, Origin::Ui).unwrap_err().status(), 400);

    let out = p.delete_reference(&r.id, Origin::Ui).unwrap();
    assert!(out.trash_id > 0);
    assert!(p.reference_by_slug("smith_1776_wealth").unwrap().is_none());
    let res = p.undo(out.trash_id, Origin::Ui).unwrap();
    assert_eq!((res.restored.as_str(), res.id.as_str()), ("reference", r.id.as_str()));
    assert!(p.reference_by_slug("smith_1776_wealth").unwrap().is_some());
    assert_eq!(p.undo(99999, Origin::Ui).unwrap_err().status(), 404);
}

#[test]
fn authors_and_tags() {
    let h = home();
    let folder = folder(h.dir.path());
    let p = scanned(&h.paths, &folder);
    let authors = p.authors().unwrap();
    let smith = authors.iter().find(|a| a.name == "Smith, A.").unwrap();
    assert_eq!(smith.hl_count, 3);
    assert!(smith.ref_ids.contains(smith.top_ref_id.as_ref().unwrap()));
    assert_eq!(authors.last().unwrap().name, "Unknown");
    let tags = p.tags().unwrap();
    let names: Vec<&str> = tags.iter().map(|t| t.name.as_str()).collect();
    for want in ["division-of-labour", "quote", "efficiency"] {
        assert!(names.contains(&want));
    }
    assert_eq!(tags.iter().find(|t| t.name == "quote").unwrap().hl_count, 1);
    let counts = p.counts().unwrap();
    assert_eq!((counts.references, counts.highlights), (7, 3));
}

#[test]
fn file_routes() {
    let h = home();
    let folder = folder(h.dir.path());
    let p = scanned(&h.paths, &folder);
    let smith = ref_by_slug(&p, "smith_1776_wealth");
    let pdf = smith.primary_file_id.clone().unwrap();
    let info = p.file_info(&pdf).unwrap();
    assert_eq!(info["ext"], json!("pdf"));
    assert_eq!(info["pages"], json!(3));
    assert_eq!(info["page_sizes"][0], json!([595.0, 842.0]));
    assert_eq!(info["rotation"], json!([0, 0, 0]));
    let png = p.render_page(&pdf, 1, 1.5).unwrap();
    assert_eq!(&png[..4], b"\x89PNG");
    assert_eq!(p.render_page(&pdf, 1, 1.5).unwrap(), png, "cached render");
    assert_eq!(p.render_page(&pdf, 9, 1.0).unwrap_err().status(), 404);
    let words = p.page_words(&pdf, 1).unwrap();
    assert_eq!(words["width"], json!(595.0));
    assert_eq!(words["rotation"], json!(0));
    assert_eq!(words["words"][0][4], json!("The"));
    assert!(p.page_text(&pdf, 1).unwrap().contains("division of labour"));
    let (raw, ctype) = p.raw(&pdf).unwrap();
    assert_eq!(&raw[..4], b"%PDF");
    assert_eq!(ctype, "application/pdf");
    assert_eq!(&p.thumb(&pdf).unwrap()[..4], b"\x89PNG");
    let md = ref_by_slug(&p, "reading").primary_file_id.unwrap();
    let (raw, ctype) = p.raw(&md).unwrap();
    assert!(ctype.starts_with("text/markdown"));
    assert!(raw.starts_with(b"# Notes"));
    assert_eq!(p.thumb(&md).unwrap_err().status(), 404);
    let info = p.file_info(&md).unwrap();
    assert_eq!(info["pages"], json!(0));
    assert_eq!(info["rotation"], json!([0]));
    let img = ref_by_slug(&p, "chart").primary_file_id.unwrap();
    let info = p.file_info(&img).unwrap();
    assert_eq!(info["page_sizes"], json!([[300, 200]]));
    assert_eq!(&p.thumb(&img).unwrap()[..4], b"\x89PNG");
    assert_eq!(p.file_info("f-000000000000").unwrap_err().status(), 404);

    let info = p.rotate(&pdf, Some(2), 90, Origin::Ui).unwrap();
    assert_eq!(info["rotation"], json!([0, 90, 0]));
    let info = p.rotate(&pdf, None, 90, Origin::Ui).unwrap();
    assert_eq!(info["rotation"], json!([90, 180, 90]));
    let info = p.rotate(&img, None, 90, Origin::Ui).unwrap();
    assert_eq!(info["rotation"], json!([90]));
    assert_eq!(p.rotate(&img, None, 45, Origin::Ui).unwrap_err().status(), 400);
    // The rotation went into the PDF itself and the scan sees the app's own write as unchanged.
    assert_eq!(p.scan(ScanMode::Incremental, |_| {}).unwrap().changed, 0);
    assert_eq!(p.file_info(&pdf).unwrap()["rotation"], json!([90, 180, 90]));
}

#[test]
fn highlight_routes() {
    let h = home();
    let folder = folder(h.dir.path());
    let p = scanned(&h.paths, &folder);
    let smith = ref_by_slug(&p, "smith_1776_wealth");
    let pdf = smith.primary_file_id.clone().unwrap();
    let words = p.page_words(&pdf, 3).unwrap();
    let w = words["words"].as_array().unwrap();
    let (w0, w1) = (&w[0], &w[1]);
    let quad = vec![w0[0].as_f64().unwrap(), w0[1].as_f64().unwrap(), w1[2].as_f64().unwrap(), w1[3].as_f64().unwrap()];
    let hl = p
        .add_highlight(
            &NewHighlight {
                file_id: pdf.clone(),
                page: Some(3),
                kind: Some("text".into()),
                quads: Some(vec![quad]),
                verbatim: Some(String::new()),
                analysis: Some("(I) Mine.".into()),
                label: None,
                tags: Some(vec!["#method".into()]),
                ..Default::default()
            },
            &Writer::human(),
        )
        .unwrap();
    assert_eq!(
        (hl.author.as_str(), hl.origin.as_str(), hl.kind.as_str(), hl.verbatim.as_str(), hl.label.as_deref(), hl.tags.clone()),
        ("You", "human", "text", "Measured tasks", Some("I"), vec!["method".to_string()])
    );
    let hls = p.highlights(&HighlightFilter { ref_id: Some(smith.id.clone()), ..Default::default() }).unwrap();
    assert_eq!(hls.len(), 4);
    assert!(hls.iter().any(|x| x.id == hl.id));
    let h2 = p
        .update_highlight(&hl.id, &HighlightPatch { analysis: Some("(P) Changed.".into()), label: Some(Some("P".into())), tags: Some(vec!["method".into(), "quote".into()]), ..Default::default() }, Origin::Ui)
        .unwrap();
    assert_eq!((h2.analysis.as_str(), h2.label.as_deref()), ("(P) Changed.", Some("P")));
    assert_eq!(p.update_highlight(&hl.id, &HighlightPatch { label: Some(Some("X".into())), ..Default::default() }, Origin::Ui).unwrap_err().status(), 400);
    // The scan sees no outside change after the app wrote the PDF.
    assert_eq!(p.scan(ScanMode::Incremental, |_| {}).unwrap().changed, 0);

    let md = ref_by_slug(&p, "reading").primary_file_id.unwrap();
    let mh = p
        .add_highlight(
            &NewHighlight {
                file_id: md.clone(),
                kind: Some("md".into()),
                anchor: Some(json!({"text": "plain sentence", "occurrence": 0})),
                verbatim: Some("plain sentence".into()),
                analysis: Some("(F) Md.".into()),
                tags: Some(vec!["quote".into()]),
                ..Default::default()
            },
            &Writer::human(),
        )
        .unwrap();
    assert_eq!(mh.kind, "md");
    assert!(mh.quads.is_empty());
    assert_eq!(mh.anchor.unwrap()["occurrence"], json!(0));
    let img = ref_by_slug(&p, "chart").primary_file_id.unwrap();
    let ih = p
        .add_highlight(
            &NewHighlight { file_id: img.clone(), kind: Some("region".into()), quads: Some(vec![vec![0.1, 0.2, 0.3, 0.4]]), verbatim: Some("Bars".into()), analysis: Some("(F) Image.".into()), tags: Some(vec![]), ..Default::default() },
            &Writer::human(),
        )
        .unwrap();
    assert_eq!(ih.quads, vec![vec![0.1, 0.2, 0.3, 0.4]]);
    assert_eq!(p.add_highlight(&NewHighlight { file_id: md.clone(), kind: Some("region".into()), ..Default::default() }, &Writer::human()).unwrap_err().status(), 400);

    let out = p.delete_highlight(&ih.id, Origin::Ui).unwrap();
    assert!(out.trash_id > 0);
    assert_eq!(p.undo(out.trash_id, Origin::Ui).unwrap().restored, "highlight");
    let out = p.delete_highlight(&hl.id, Origin::Ui).unwrap();
    assert_eq!(ref_by_slug(&p, "smith_1776_wealth").hl_count, 3);
    p.undo(out.trash_id, Origin::Ui).unwrap();
    let restored = p.highlights(&HighlightFilter { file_id: Some(pdf.clone()), ..Default::default() }).unwrap();
    assert!(restored.iter().any(|x| x.analysis == "(P) Changed."));
    // The restored highlight is back in the PDF as one annotation: the two Claude highlights
    // of the translation plus the restored one.
    assert_eq!(p.scan(ScanMode::Full, |_| {}).unwrap().highlights_added, 0);
    assert_eq!(p.highlights(&HighlightFilter { file_id: Some(pdf), ..Default::default() }).unwrap().len(), 3);
}

#[test]
fn quote_highlight_and_ai_writer() {
    let h = home();
    let folder = folder(h.dir.path());
    let p = scanned(&h.paths, &folder);
    let smith = ref_by_slug(&p, "smith_1776_wealth");
    let pdf = smith.files.iter().find(|f| f.role == "print").unwrap().id.clone();
    let hl = p.add_quote_highlight(&pdf, 3, "bonus link pay", "Pay follows output.", Some("I"), &["incentives".into()], &Writer::ai("Claude")).unwrap();
    assert_eq!((hl.author.as_str(), hl.origin.as_str(), hl.label.as_deref()), ("Claude", "ai", Some("I")));
    assert_eq!(hl.verbatim, "bonus link pay");
    assert_eq!(hl.tags, vec!["incentives"]);
    assert_eq!(p.add_quote_highlight(&pdf, 3, "words that are absent", "x", None, &[], &Writer::ai("Claude")).unwrap_err().status(), 404);
    let last = p.changes_since(0).unwrap().changes.last().unwrap().clone();
    assert_eq!(last.origin, "mcp");
}

#[test]
fn suggestions_and_accept_tag() {
    let h = home();
    let folder = folder(h.dir.path());
    let p = scanned(&h.paths, &folder);
    let smith = ref_by_slug(&p, "smith_1776_wealth");
    let hl = p.highlights(&HighlightFilter { ref_id: Some(smith.id.clone()), ..Default::default() }).unwrap().remove(0);
    p.suggest_tags(&hl.id, &["method".into(), "to-read".into()], Origin::Mcp).unwrap();
    let out = p.accept_tag(&hl.id, "method", false, Origin::Ui).unwrap();
    assert!(out.tags.contains(&"method".to_string()));
    assert_eq!(out.suggested_tags, vec!["to-read"]);
    let out = p.accept_tag(&hl.id, "to-read", true, Origin::Ui).unwrap();
    assert!(!out.tags.contains(&"to-read".to_string()));
    assert!(out.suggested_tags.is_empty());
    assert_eq!(p.resolve_suggestion(&hl.id, "accept", Origin::Ui).unwrap_err().status(), 400);
    p.propose_delete(&hl.id, "Duplicate.", Origin::Mcp).unwrap();
    let out = p.resolve_suggestion(&hl.id, "dismiss", Origin::Ui).unwrap();
    assert_eq!(out["highlight"]["suggestion"]["resolution"], json!("dismissed"));
    p.propose_delete(&hl.id, "Duplicate again.", Origin::Mcp).unwrap();
    let out = p.resolve_suggestion(&hl.id, "accept", Origin::Ui).unwrap();
    assert_eq!(out["deleted"], json!(true));
    assert!(out["trash_id"].as_i64().unwrap() > 0);
    assert!(p.highlights(&HighlightFilter { file_id: Some(hl.file_id.clone()), ..Default::default() }).unwrap().iter().all(|x| x.id != hl.id));
}

#[test]
fn settings_read_only_and_locked_writes() {
    let h = home();
    let folder = folder(h.dir.path());
    let p = scanned(&h.paths, &folder);
    let s = p.update_settings(&fields(&[("read_only", json!(true)), ("auto_scan_min", json!(5)), ("theme", json!("dark")), ("name", json!("Renamed"))]), Origin::Ui).unwrap();
    assert_eq!(s["read_only"], json!(true));
    assert_eq!(s["auto_scan_min"], json!(5));
    assert_eq!(p.name(), "Renamed");
    assert_eq!(h.paths.resolve(p.id()).unwrap().name, "Renamed");
    assert_eq!(p.update_settings(&fields(&[("auto_scan_min", json!("x"))]), Origin::Ui).unwrap_err().status(), 400);
    assert_eq!(p.update_settings(&fields(&[("folder", json!("C:\\elsewhere"))]), Origin::Ui).unwrap_err().status(), 400);
    let pdf = ref_by_slug(&p, "smith_1776_wealth").primary_file_id.unwrap();
    let err = p.add_highlight(&NewHighlight { file_id: pdf.clone(), page: Some(1), quads: Some(vec![vec![72.0, 72.0, 200.0, 90.0]]), analysis: Some("x".into()), ..Default::default() }, &Writer::human()).unwrap_err();
    assert_eq!(err.status(), 423);
    assert!(err.message().contains("read-only"));
    assert_eq!(p.rotate(&pdf, Some(1), 90, Origin::Ui).unwrap_err().status(), 423);
    assert_eq!(p.export_references("apa-md", None).unwrap_err().status(), 423);
    assert_eq!(p.export_ai_log().unwrap_err().status(), 423);
    // Database writes still happen.
    let chart = ref_by_slug(&p, "chart");
    let (upd, _) = p.update_reference(&chart.id, &ReferencePatch { fields: fields(&[("title", json!("Chart (read-only edit)"))]), ..Default::default() }, Origin::Ui, "Verified").unwrap();
    assert_eq!(upd.title, "Chart (read-only edit)");
    p.set_read_only(false).unwrap();
    assert!(!p.read_only());
}

#[test]
fn changes_ai_log_export_and_resolve() {
    let h = home();
    let folder = folder(h.dir.path());
    let p = scanned(&h.paths, &folder);
    let ch = p.changes_since(0).unwrap();
    assert!(ch.seq >= 1);
    assert!(["scan", "ui"].contains(&ch.changes[0].origin.as_str()));
    let seq = ch.seq;
    let mut rx = p.subscribe();
    let chart = ref_by_slug(&p, "chart");
    p.update_reference(&chart.id, &ReferencePatch { fields: fields(&[("title", json!("Chart two"))]), ..Default::default() }, Origin::Ui, "Verified").unwrap();
    let ch = p.changes_since(seq).unwrap();
    assert_eq!(ch.changes.iter().map(|c| c.origin.as_str()).collect::<Vec<_>>(), vec!["ui"]);
    assert!(ch.changes[0].ids.contains(&chart.id));
    assert_eq!(rx.try_recv().unwrap(), ch.seq);
    assert_eq!(p.change_seq(), ch.seq);

    p.log_ai("catalog_scan", "claude-code", "Rescan", &json!({"full": false}), "0 new", None).unwrap();
    p.log_ai("log_append", "claude-code", "Checked a quote", &json!({"checks": "Compared with page 3 | ok"}), "ok", None).unwrap();
    let log = p.ai_log().unwrap();
    assert_eq!(log.iter().map(|x| x.tool.as_str()).collect::<Vec<_>>(), vec!["log_append", "catalog_scan"]);
    let path = p.export_ai_log().unwrap();
    assert_eq!(path, folder.join(".kurasi").join("AI_USE_LOG.md"));
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("| Date | Tool | Purpose | Checks performed by the author |"));
    assert!(text.contains("Compared with page 3 \\| ok"));

    let smith = ref_by_slug(&p, "smith_1776_wealth");
    for (fmt, name) in [("apa-md", "references_apa.md"), ("bibtex", "references.bib"), ("csl-json", "references.csl.json"), ("highlights-json", "highlights.json")] {
        let out = p.export_references(fmt, Some(&[smith.id.clone()])).unwrap();
        assert_eq!(out, folder.join("_exports").join(name));
        assert!(std::fs::metadata(&out).unwrap().len() > 10);
    }
    let apa = std::fs::read_to_string(folder.join("_exports").join("references_apa.md")).unwrap();
    assert!(apa.contains("Smith, A. (1776). *The wealth of nations*."), "{}", apa);
    let csl: Value = serde_json::from_str(&std::fs::read_to_string(folder.join("_exports").join("references.csl.json")).unwrap()).unwrap();
    assert_eq!(csl[0]["author"], json!([{"family": "Smith", "given": "A."}]));
    let hl: Value = serde_json::from_str(&std::fs::read_to_string(folder.join("_exports").join("highlights.json")).unwrap()).unwrap();
    assert_eq!(hl["highlights"].as_array().unwrap().len(), 3);
    assert_eq!(p.export_references("docx", None).unwrap_err().status(), 400);

    let md = ref_by_slug(&p, "reading").primary_file_id.unwrap();
    let fid = chart.primary_file_id.clone().unwrap();
    let res = p.resolve("../lib/smith_1776_wealth.pdf", Some(&md)).unwrap();
    assert_eq!(res["ref_id"], json!(smith.id));
    let res = p.resolve("img/chart.png#top", Some(&md)).unwrap();
    assert_eq!(res, json!({"file_id": fid, "ref_id": chart.id}));
    assert_eq!(p.resolve("https://example.org/page", Some(&md)).unwrap(), json!({"external": true}));
    let res = p.resolve("../notes/skip.txt", Some(&md)).unwrap();
    assert_eq!(res["external"], json!(true));
    assert_eq!(res["relpath"], json!("notes\\skip.txt"));
    let exe = std::env::current_exe().unwrap();
    assert_eq!(
        h.paths.mcp_command(&folder),
        format!("claude mcp add kurasi -s user -- \"{}\" mcp --project \"{}\" --mode write", exe.display(), folder.display())
    );
}

#[test]
fn document_notes_on_every_file_type() {
    let h = home();
    let folder = folder(h.dir.path());
    let p = scanned(&h.paths, &folder);
    let pdf = ref_by_slug(&p, "smith_1776_wealth").primary_file_id.unwrap();
    let md = ref_by_slug(&p, "reading").primary_file_id.unwrap();
    let img = ref_by_slug(&p, "chart").primary_file_id.unwrap();
    for fid in [&pdf, &md, &img] {
        let hl = p
            .add_highlight(
                &NewHighlight { file_id: fid.clone(), page: None, kind: Some("note".into()), quads: Some(vec![]), verbatim: Some(String::new()), analysis: Some("(P) Note on the file.".into()), label: Some(String::new()), tags: Some(vec!["to-read".into()]), ..Default::default() },
                &Writer::human(),
            )
            .unwrap();
        assert_eq!((hl.kind.as_str(), hl.label.as_deref(), hl.tags.clone()), ("note", Some("P"), vec!["to-read".to_string()]));
    }
    let notes: Vec<_> = p.highlights(&HighlightFilter { file_id: Some(pdf.clone()), ..Default::default() }).unwrap().into_iter().filter(|x| x.kind == "note").collect();
    assert_eq!(notes[0].page, Some(1));
    // The PDF note is a real annotation: a full rescan keeps exactly one copy of it.
    p.scan(ScanMode::Full, |_| {}).unwrap();
    let again: Vec<String> = p.highlights(&HighlightFilter { file_id: Some(pdf), ..Default::default() }).unwrap().into_iter().filter(|x| x.kind == "note").map(|x| x.id).collect();
    assert_eq!(again, vec![notes[0].id.clone()]);
}

#[test]
fn search_ranks_the_typo_query_first() {
    let h = home();
    let folder = folder(h.dir.path());
    let p = scanned(&h.paths, &folder);
    let hits = p.search("welth nations", true, 10).unwrap();
    assert!(!hits.is_empty());
    assert_eq!(hits[0].reference.slug, "smith_1776_wealth");
    assert!(hits[0].score > 0.0);
    let strict = p.search("zzzz qqqq", false, 10).unwrap();
    assert!(strict.is_empty());
    // An empty query matches every reference with the same score.
    assert_eq!(p.search("", true, 10).unwrap().len(), 7);
}

#[test]
fn categorize_and_setting_helpers() {
    let h = home();
    let folder = folder(h.dir.path());
    let p = scanned(&h.paths, &folder);
    let a = ref_by_slug(&p, "a/_NOTES");
    let b = ref_by_slug(&p, "b/_NOTES");
    let out = p.categorize(&[a.id.clone(), b.id.clone()], &["Notes/Folders".into()], Origin::Mcp, "Suggested").unwrap();
    assert_eq!(out.len(), 2);
    assert!(out.iter().all(|r| r.categories == vec!["Notes/Folders"] && r.status == "Suggested"));
    assert_eq!(p.setting_str("name"), "Test project");
    assert_eq!(p.setting_strs("file_types"), vec![".pdf", ".md", ".png", ".jpg", ".jpeg"]);
    assert!(p.settings().contains_key("exclude"));
    let info = p.project_info();
    assert_eq!(info["id"], json!(p.id()));
}
