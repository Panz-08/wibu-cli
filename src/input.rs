use dialoguer::{
    console::{style, Style},
    theme::ColorfulTheme,
    FuzzySelect, Input, Select,
};

use crate::models::SearchMode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuAction {
    Search,
    Browse(SearchMode),
    Japan(SearchMode),
    Quit,
}

fn theme() -> ColorfulTheme {
    ColorfulTheme {
        prompt_prefix: style("◆".to_string()).for_stderr().cyan(),
        success_prefix: style("◇".to_string()).for_stderr().green(),
        error_prefix: style("✖".to_string()).for_stderr().red(),
        prompt_suffix: style("›".to_string()).for_stderr().dim(),
        active_item_prefix: style("❯".to_string()).for_stderr().cyan(),
        inactive_item_prefix: style(" ".to_string()).for_stderr(),
        active_item_style: Style::new().cyan().bold(),
        inactive_item_style: Style::new().white(),
        prompt_style: Style::new().white(),
        values_style: Style::new().green(),
        hint_style: Style::new().dim(),
        ..ColorfulTheme::default()
    }
}

pub fn main_menu() -> anyhow::Result<MenuAction> {
    let items = vec![
        "Cari anime",
        "Anime ongoing",
        "Anime selesai",
        "Film anime",
        "Film Jepang",
        "Series & drama Jepang",
        "Tokusatsu (Ultraman, Kamen Rider, Sentai)",
        "Keluar",
    ];

    let selected = Select::with_theme(&theme())
        .with_prompt("Mau nonton apa?")
        .default(0)
        .items(&items)
        .interact()?;

    Ok(match selected {
        0 => MenuAction::Search,
        1 => MenuAction::Browse(SearchMode::Ongoing),
        2 => MenuAction::Browse(SearchMode::Completed),
        3 => MenuAction::Browse(SearchMode::Movie),
        4 => MenuAction::Japan(SearchMode::JapanMovie),
        5 => MenuAction::Japan(SearchMode::JapanSeries),
        6 => MenuAction::Japan(SearchMode::Tokusatsu),
        _ => MenuAction::Quit,
    })
}

pub fn search_prompt(prompt: &str) -> anyhow::Result<String> {
    let mut title = String::new();
    while title.trim().is_empty() {
        title = Input::with_theme(&theme())
            .with_prompt(prompt)
            .interact_text()?;
    }

    Ok(title.trim().to_string())
}

/// Prompt opsional: kosong berarti jelajahi daftar tanpa kata kunci.
pub fn optional_query(prompt: &str) -> anyhow::Result<Option<String>> {
    let value: String = Input::with_theme(&theme())
        .with_prompt(prompt)
        .allow_empty(true)
        .interact_text()?;

    let value = value.trim().to_string();
    Ok(if value.is_empty() { None } else { Some(value) })
}

pub fn choice<T: std::fmt::Display + Clone>(
    options: Vec<T>,
    prompt: &str,
    fuzzy: bool,
) -> anyhow::Result<T> {
    anyhow::ensure!(!options.is_empty(), "Tidak ada pilihan yang tersedia");

    let selected = if fuzzy {
        FuzzySelect::with_theme(&theme())
            .with_prompt(prompt)
            .default(0)
            .max_length(12)
            .items(&options)
            .interact()?
    } else {
        Select::with_theme(&theme())
            .with_prompt(prompt)
            .default(0)
            .max_length(12)
            .items(&options)
            .interact()?
    };

    Ok(options[selected].clone())
}

pub fn confirm(prompt: &str, default: bool) -> anyhow::Result<bool> {
    Ok(dialoguer::Confirm::with_theme(&theme())
        .with_prompt(prompt)
        .default(default)
        .interact()?)
}
