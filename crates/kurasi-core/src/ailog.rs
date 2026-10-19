//! The AI-use log: one row per MCP tool call, newest first when read, and its Markdown export.

use rusqlite::{params, Connection};
use serde_json::Value;

use crate::db::{dumps, loads};
use crate::error::Result;
use crate::model::AiLogRow;
use crate::util::now_iso;

/// Adds a row. `at` defaults to now; `result` is cut to 2000 characters.
pub fn log_ai(c: &Connection, tool: &str, client: &str, purpose: &str, args: &Value, result: &str, at: Option<&str>) -> Result<i64> {
    let client = if client.trim().is_empty() { "unknown client" } else { client };
    let result: String = result.trim().chars().take(2000).collect();
    c.execute(
        "INSERT INTO ai_log(at,tool,client,purpose,args,result) VALUES(?,?,?,?,?,?)",
        params![at.map(str::to_string).unwrap_or_else(now_iso), tool, client, purpose.trim(), dumps(args), result],
    )?;
    Ok(c.last_insert_rowid())
}

/// Every row, newest first.
pub fn ai_log(c: &Connection) -> Result<Vec<AiLogRow>> {
    let mut stmt = c.prepare("SELECT id,at,tool,client,purpose,args,result FROM ai_log ORDER BY at DESC, id DESC")?;
    let rows = stmt.query_map([], |r| {
        Ok(AiLogRow {
            id: r.get(0)?,
            at: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
            tool: r.get::<_, Option<String>>(2)?.unwrap_or_default(),
            client: r.get::<_, Option<String>>(3)?.unwrap_or_default(),
            purpose: r.get::<_, Option<String>>(4)?.unwrap_or_default(),
            args: loads(r.get::<_, Option<String>>(5)?.as_deref()).unwrap_or_else(|| Value::Object(Default::default())),
            result: r.get::<_, Option<String>>(6)?.unwrap_or_default(),
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn cell(text: &str) -> String {
    crate::util::norm_space(text).replace('|', "\\|")
}

/// The Markdown table `| Date | Tool | Purpose | Checks performed by the author |`, oldest first.
pub fn render_markdown(project_name: &str, rows_newest_first: &[AiLogRow]) -> String {
    let mut lines = vec![
        "# AI use log".to_string(),
        String::new(),
        format!("Exported by Kurasi from the project {} on {}.", project_name, now_iso().replace('T', " ")),
        String::new(),
        "| Date | Tool | Purpose | Checks performed by the author |".to_string(),
        "| --- | --- | --- | --- |".to_string(),
    ];
    for r in rows_newest_first.iter().rev() {
        let checks = r.args.get("checks").and_then(|v| v.as_str()).unwrap_or("");
        let tool = format!("{}, Kurasi MCP {}", r.client, r.tool);
        let date: String = r.at.replace('T', " ").chars().take(16).collect();
        let purpose = if r.purpose.is_empty() { r.result.as_str() } else { r.purpose.as_str() };
        lines.push(format!("| {} | {} | {} | {} |", cell(&date), cell(&tool), cell(purpose), cell(checks)));
    }
    lines.join("\n") + "\n"
}
