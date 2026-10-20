#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod geom;
mod gfx;
mod model;
mod platform;
mod view;

use std::path::PathBuf;
use std::process::ExitCode;

use app::Launch;
use model::data_path;

fn main() -> ExitCode {
    platform::attach_console();
    match app::run(parse_args()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            platform::error_box(&error);
            ExitCode::from(1)
        }
    }
}

fn parse_args() -> Launch {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut screenshot = None;
    let mut now = None;
    let mut date = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--screenshot" => {
                index += 1;
                screenshot = args.get(index).map(PathBuf::from);
            }
            "--now" => {
                index += 1;
                now = args.get(index).and_then(|text| parse_clock(text));
            }
            "--date" => {
                index += 1;
                date = args.get(index).and_then(|text| parse_date(text));
            }
            "--data" => {
                index += 1;
            }
            other => eprintln!("忽略未知参数 {other}"),
        }
        index += 1;
    }
    Launch {
        data: data_arg(&args).unwrap_or_else(data_path),
        now,
        date,
        screenshot,
    }
}

fn data_arg(args: &[String]) -> Option<PathBuf> {
    args.windows(2).find_map(|pair| {
        if pair[0] == "--data" {
            Some(PathBuf::from(&pair[1]))
        } else {
            None
        }
    })
}

fn parse_clock(text: &str) -> Option<f32> {
    let (hour, minute) = text.split_once([':', '：'])?;
    let hour: f32 = hour.parse().ok()?;
    let minute: f32 = minute.parse().ok()?;
    if !(0.0..24.0).contains(&hour) || !(0.0..60.0).contains(&minute) {
        return None;
    }
    Some(hour * 60.0 + minute)
}

fn parse_date(text: &str) -> Option<(i32, u32, u32)> {
    let mut parts = text.split('-');
    let year = parts.next()?.parse().ok()?;
    let month = parts.next()?.parse().ok()?;
    let day = parts.next()?.parse().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    Some((year, month, day))
}
