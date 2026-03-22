use anyhow::{Context, Result};
use chrono::{NaiveDate, Utc};
use std::env;

fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();
    let date = if args.len() >= 2 {
        NaiveDate::parse_from_str(&args[1], "%Y-%m-%d")
            .with_context(|| format!("Invalid date: {}", &args[1]))?
    } else {
        Utc::now().date_naive()
    };
    let result = tradingbots_fun::reporting::refresh_reports(date)?;
    println!("Report written: {}", result.journal_path.display());
    println!(
        "Pattern summary written: {}",
        result.pattern_json_path.display()
    );
    println!(
        "Pattern journal snapshot: {}",
        result.pattern_md_path.display()
    );
    Ok(())
}
