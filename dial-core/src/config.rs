use crate::db::get_db;
use crate::errors::Result;
use crate::output::bold;
use chrono::Local;

pub fn config_get(key: &str) -> Result<Option<String>> {
    let conn = get_db(None)?;
    let mut stmt = conn.prepare("SELECT value FROM config WHERE key = ?1")?;
    let result = stmt.query_row([key], |row| row.get(0)).ok();
    Ok(result)
}

pub fn config_set(key: &str, value: &str) -> Result<()> {
    let conn = get_db(None)?;
    let now = Local::now().to_rfc3339();
    conn.execute(
        "INSERT INTO config (key, value, updated_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(key) DO UPDATE SET value = ?2, updated_at = ?3",
        [key, value, &now],
    )?;
    Ok(())
}

pub fn config_show() -> Result<()> {
    let conn = get_db(None)?;
    let mut stmt = conn.prepare("SELECT key, value FROM config ORDER BY key")?;
    let rows: Vec<(String, String)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<std::result::Result<Vec<_>, _>>()?;

    println!("{}", bold("DIAL Configuration"));
    println!("{}", "=".repeat(40));
    for (key, value) in rows {
        println!("  {}: {}", key, value);
    }
    Ok(())
}
