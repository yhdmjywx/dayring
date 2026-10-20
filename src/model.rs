//! One day of tasks, stored as minutes from local midnight.

use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub const DAY: i32 = 24 * 60;
pub const DAY_F: f32 = DAY as f32;
pub const SNAP: i32 = 15;
pub const MIN_DURATION: i32 = 15;
pub const DEFAULT_DURATION: i32 = 60;
pub const COLOR_COUNT: usize = 5;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Task {
    pub id: u64,
    pub name: String,
    pub start_min: i32,
    pub end_min: i32,
    pub color: usize,
}

impl Task {
    #[allow(dead_code)]
    pub fn duration(&self) -> i32 {
        duration(self.start_min, self.end_min)
    }
}

pub fn duration(start: i32, end: i32) -> i32 {
    (end - start).rem_euclid(DAY)
}

pub fn snap_minutes(minutes: f32) -> i32 {
    let snapped = (minutes / SNAP as f32).round() as i32 * SNAP;
    snapped.rem_euclid(DAY)
}

pub fn fmt_hhmm(minutes: i32) -> String {
    let minutes = minutes.rem_euclid(DAY);
    format!("{:02}:{:02}", minutes / 60, minutes % 60)
}

pub fn parse_hhmm(text: &str) -> Option<i32> {
    let text = text.trim();
    let (hour, minute) = if let Some((hour, minute)) = text.split_once([':', '：']) {
        (hour.parse::<i32>().ok()?, minute.parse::<i32>().ok()?)
    } else if text.len() == 3 && text.bytes().all(|byte| byte.is_ascii_digit()) {
        (text[..1].parse().ok()?, text[1..].parse().ok()?)
    } else if text.len() == 4 && text.bytes().all(|byte| byte.is_ascii_digit()) {
        (text[..2].parse().ok()?, text[2..].parse().ok()?)
    } else {
        return None;
    };
    if !(0..24).contains(&hour) || !(0..60).contains(&minute) {
        return None;
    }
    Some(snap_minutes((hour * 60 + minute) as f32))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Schedule {
    tasks: Vec<Task>,
    selected: Option<u64>,
    next_id: u64,
}

#[derive(Serialize, Deserialize)]
struct FileDto {
    version: u32,
    next_id: u64,
    selected: Option<u64>,
    tasks: Vec<Task>,
}

impl Schedule {
    pub fn demo() -> Self {
        let tasks = vec![
            Task {
                id: 1,
                name: "晨间".into(),
                start_min: 7 * 60,
                end_min: 9 * 60,
                color: 0,
            },
            Task {
                id: 2,
                name: "创作".into(),
                start_min: 9 * 60,
                end_min: 11 * 60 + 30,
                color: 1,
            },
            Task {
                id: 3,
                name: "午间休息".into(),
                start_min: 12 * 60,
                end_min: 13 * 60 + 30,
                color: 4,
            },
            Task {
                id: 4,
                name: "学习".into(),
                start_min: 14 * 60,
                end_min: 17 * 60,
                color: 2,
            },
            Task {
                id: 5,
                name: "阅读".into(),
                start_min: 20 * 60,
                end_min: 22 * 60,
                color: 3,
            },
        ];
        Self {
            tasks,
            selected: Some(2),
            next_id: 6,
        }
    }

    pub fn tasks(&self) -> &[Task] {
        &self.tasks
    }

    pub fn selected_id(&self) -> Option<u64> {
        self.selected
    }

    pub fn selected_task(&self) -> Option<&Task> {
        self.selected
            .and_then(|id| self.tasks.iter().find(|task| task.id == id))
    }

    pub fn selected_task_mut(&mut self) -> Option<&mut Task> {
        let id = self.selected?;
        self.tasks.iter_mut().find(|task| task.id == id)
    }

    pub fn select(&mut self, id: u64) {
        if self.tasks.iter().any(|task| task.id == id) {
            self.selected = Some(id);
        }
    }

    pub fn select_next(&mut self) {
        if self.tasks.is_empty() {
            self.selected = None;
            return;
        }
        let mut order: Vec<u64> = self.tasks.iter().map(|task| task.id).collect();
        order.sort_by_key(|id| {
            let task = self.tasks.iter().find(|task| task.id == *id).unwrap();
            (task.start_min, task.id)
        });
        let next = match self.selected {
            Some(current) => {
                let index = order.iter().position(|id| *id == current).unwrap_or(0);
                order[(index + 1) % order.len()]
            }
            None => order[0],
        };
        self.selected = Some(next);
    }

    pub fn add_task(&mut self, start_min: i32) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        let start_min = snap_minutes(start_min as f32);
        let end_min = (start_min + DEFAULT_DURATION).rem_euclid(DAY);
        let color = self.tasks.len() % COLOR_COUNT;
        self.tasks.push(Task {
            id,
            name: "新安排".into(),
            start_min,
            end_min,
            color,
        });
        self.selected = Some(id);
        id
    }

    pub fn delete_selected(&mut self) -> bool {
        let Some(id) = self.selected else {
            return false;
        };
        let Some(index) = self.tasks.iter().position(|task| task.id == id) else {
            return false;
        };
        self.tasks.remove(index);
        self.selected = self
            .tasks
            .iter()
            .min_by_key(|task| (task.start_min, task.id))
            .map(|task| task.id);
        true
    }

    pub fn set_name(&mut self, name: String) -> bool {
        let Some(task) = self.selected_task_mut() else {
            return false;
        };
        if task.name == name {
            return false;
        }
        task.name = name;
        true
    }

    pub fn set_color(&mut self, color: usize) -> bool {
        let Some(task) = self.selected_task_mut() else {
            return false;
        };
        let color = color % COLOR_COUNT;
        if task.color == color {
            return false;
        }
        task.color = color;
        true
    }

    pub fn set_start(&mut self, minutes: i32) -> bool {
        let Some(task) = self.selected_task() else {
            return false;
        };
        let end = task.end_min;
        let previous = task.start_min;
        let start = crate::geom::clamp_start(snap_minutes(minutes as f32), end, previous);
        if start == previous {
            return false;
        }
        self.selected_task_mut().unwrap().start_min = start;
        true
    }

    pub fn set_end(&mut self, minutes: i32) -> bool {
        let Some(task) = self.selected_task() else {
            return false;
        };
        let start = task.start_min;
        let previous = task.end_min;
        let end = crate::geom::clamp_end(snap_minutes(minutes as f32), start, previous);
        if end == previous {
            return false;
        }
        self.selected_task_mut().unwrap().end_min = end;
        true
    }

    /// Slide the whole task. Duration, including a midnight wrap, stays the same.
    pub fn move_by(&mut self, id: u64, from_start: i32, from_end: i32, delta_minutes: f32) -> bool {
        let Some(task) = self.tasks.iter_mut().find(|task| task.id == id) else {
            return false;
        };
        let span = duration(from_start, from_end).max(MIN_DURATION);
        let start = snap_minutes(from_start as f32 + delta_minutes);
        let end = (start + span).rem_euclid(DAY);
        if task.start_min == start && task.end_min == end {
            return false;
        }
        task.start_min = start;
        task.end_min = end;
        true
    }

    pub fn resize_start(&mut self, id: u64, minutes: f32) -> bool {
        let Some(task) = self.tasks.iter().find(|task| task.id == id) else {
            return false;
        };
        let end = task.end_min;
        let previous = task.start_min;
        let start = crate::geom::clamp_start(snap_minutes(minutes), end, previous);
        if start == previous {
            return false;
        }
        self.tasks.iter_mut().find(|task| task.id == id).unwrap().start_min = start;
        true
    }

    pub fn resize_end(&mut self, id: u64, minutes: f32) -> bool {
        let Some(task) = self.tasks.iter().find(|task| task.id == id) else {
            return false;
        };
        let start = task.start_min;
        let previous = task.end_min;
        let end = crate::geom::clamp_end(snap_minutes(minutes), start, previous);
        if end == previous {
            return false;
        }
        self.tasks.iter_mut().find(|task| task.id == id).unwrap().end_min = end;
        true
    }

    pub fn tasks_by_start(&self) -> Vec<&Task> {
        let mut tasks: Vec<&Task> = self.tasks.iter().collect();
        tasks.sort_by_key(|task| (task.start_min, task.id));
        tasks
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let dto = FileDto {
            version: 1,
            next_id: self.next_id,
            selected: self.selected,
            tasks: self.tasks.clone(),
        };
        let json = serde_json::to_string_pretty(&dto).map_err(io::Error::other)?;
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, json)?;
        fs::rename(&tmp, path)?;
        Ok(())
    }

    pub fn load_or_demo(path: &Path) -> Self {
        if !path.exists() {
            let schedule = Self::demo();
            let _ = schedule.save(path);
            return schedule;
        }
        match fs::read_to_string(path) {
            Ok(text) => match serde_json::from_str::<FileDto>(&text) {
                Ok(dto) => dto.into_schedule(),
                Err(_) => {
                    let backup = path.with_extension("json.bak");
                    let _ = fs::rename(path, backup);
                    let schedule = Self::demo();
                    let _ = schedule.save(path);
                    schedule
                }
            },
            Err(_) => Self::demo(),
        }
    }
}

impl FileDto {
    fn into_schedule(self) -> Schedule {
        let mut tasks = Vec::new();
        let mut max_id = 0;
        for mut task in self.tasks {
            task.start_min = task.start_min.rem_euclid(DAY);
            task.end_min = task.end_min.rem_euclid(DAY);
            task.color %= COLOR_COUNT;
            if duration(task.start_min, task.end_min) < MIN_DURATION {
                task.end_min = (task.start_min + DEFAULT_DURATION).rem_euclid(DAY);
            }
            max_id = max_id.max(task.id);
            tasks.push(task);
        }
        let selected = self
            .selected
            .filter(|id| tasks.iter().any(|task| task.id == *id))
            .or_else(|| tasks.first().map(|task| task.id));
        Schedule {
            tasks,
            selected,
            next_id: self.next_id.max(max_id.saturating_add(1)).max(1),
        }
    }
}

pub fn data_path() -> PathBuf {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Dayring")
        .join("schedule.json")
}

/// Year, month, day, and minutes from local midnight including fractional seconds.
pub fn local_now() -> (i32, u32, u32, f32) {
    let stamp = system_time();
    (
        stamp.year as i32,
        stamp.month as u32,
        stamp.day as u32,
        stamp.hour as f32 * 60.0 + stamp.minute as f32 + stamp.second as f32 / 60.0,
    )
}

pub fn format_date(year: i32, month: u32, day: u32) -> String {
    let weekday = weekday_index(year, month, day);
    const NAMES: [&str; 7] = ["日", "一", "二", "三", "四", "五", "六"];
    format!("{month}月{day}日星期{}", NAMES[weekday])
}

fn weekday_index(year: i32, month: u32, day: u32) -> usize {
    // Sakamoto's method. Sunday is 0.
    const T: [i32; 12] = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let mut year = year;
    if month < 3 {
        year -= 1;
    }
    let month = month as usize;
    let index = (year + year / 4 - year / 100 + year / 400 + T[month - 1] + day as i32) % 7;
    index.rem_euclid(7) as usize
}

#[cfg(windows)]
fn system_time() -> SystemTimeParts {
    #[repr(C)]
    struct SystemTime {
        year: u16,
        month: u16,
        dow: u16,
        day: u16,
        hour: u16,
        minute: u16,
        second: u16,
        millis: u16,
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetLocalTime(time: *mut SystemTime);
    }
    let mut time = SystemTime {
        year: 0,
        month: 0,
        dow: 0,
        day: 0,
        hour: 0,
        minute: 0,
        second: 0,
        millis: 0,
    };
    unsafe { GetLocalTime(&mut time) };
    SystemTimeParts {
        year: time.year,
        month: time.month,
        day: time.day,
        hour: time.hour,
        minute: time.minute,
        second: time.second,
    }
}

#[cfg(not(windows))]
fn system_time() -> SystemTimeParts {
    SystemTimeParts {
        year: 2026,
        month: 10,
        day: 19,
        hour: 9,
        minute: 50,
        second: 0,
    }
}

struct SystemTimeParts {
    year: u16,
    month: u16,
    day: u16,
    hour: u16,
    minute: u16,
    second: u16,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::{clamp_end, clamp_start};

    #[test]
    fn snap_and_parse_use_fifteen_minutes() {
        assert_eq!(snap_minutes(9.0 * 60.0 + 7.0), 9 * 60);
        assert_eq!(snap_minutes(9.0 * 60.0 + 8.0), 9 * 60 + 15);
        assert_eq!(snap_minutes(-1.0), 0);
        assert_eq!(parse_hhmm("09:07"), Some(9 * 60));
        assert_eq!(parse_hhmm("23:55"), Some(0));
        assert_eq!(parse_hhmm("9：15"), Some(9 * 60 + 15));
        assert_eq!(parse_hhmm("24:00"), None);
    }

    #[test]
    fn move_preserves_duration_across_midnight() {
        let mut schedule = Schedule::demo();
        schedule.add_task(23 * 60);
        let id = schedule.selected_id().unwrap();
        schedule.selected_task_mut().unwrap().end_min = 60;
        assert!(schedule.move_by(id, 23 * 60, 60, 30.0));
        let task = schedule.selected_task().unwrap();
        assert_eq!(task.start_min, 23 * 60 + 30);
        assert_eq!(task.end_min, 90);
        assert_eq!(task.duration(), 120);
    }

    #[test]
    fn form_edit_rejects_zero_length() {
        let mut schedule = Schedule::demo();
        let before = schedule.selected_task().unwrap().clone();
        assert!(schedule.set_end(before.start_min));
        assert_eq!(schedule.selected_task().unwrap().duration(), MIN_DURATION);
        assert!(!schedule.set_end(before.start_min));
    }

    #[test]
    fn json_roundtrip_keeps_wrap_and_selection() {
        let dir = std::env::temp_dir().join(format!("dayring-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("schedule.json");
        let mut schedule = Schedule::demo();
        schedule.add_task(23 * 60);
        let id = schedule.selected_id().unwrap();
        schedule.selected_task_mut().unwrap().end_min = 60;
        schedule.save(&path).unwrap();
        let loaded = Schedule::load_or_demo(&path);
        let task = loaded.tasks().iter().find(|task| task.id == id).unwrap();
        assert_eq!((task.start_min, task.end_min), (23 * 60, 60));
        assert_eq!(loaded.selected_id(), Some(id));
        assert_eq!(loaded.tasks().len(), 6);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_file_seeds_demo_once() {
        let dir = std::env::temp_dir().join(format!("dayring-seed-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let path = dir.join("schedule.json");
        let first = Schedule::load_or_demo(&path);
        assert_eq!(first.tasks().len(), 5);
        assert_eq!(first.selected_task().unwrap().name, "创作");
        assert!(path.exists());
        let mut edited = Schedule::load_or_demo(&path);
        edited.delete_selected();
        edited.save(&path).unwrap();
        let again = Schedule::load_or_demo(&path);
        assert_eq!(again.tasks().len(), 4);
        assert!(again.tasks().iter().all(|task| task.name != "创作"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn monday_19_october_2026() {
        assert_eq!(format_date(2026, 10, 19), "10月19日星期一");
    }

    #[test]
    fn clamp_helpers_match_schedule_edits() {
        assert_eq!(clamp_start(10 * 60, 10 * 60, 9 * 60), 10 * 60 - 15);
        assert_eq!(clamp_end(9 * 60, 9 * 60, 10 * 60), 9 * 60 + 15);
    }
}
