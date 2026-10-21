//! Win32 window, input, and timers. The window repaints when the clock ticks,
//! the pointer moves onto a new control, or the schedule changes.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use windows::{
    Win32::{
        Foundation::*,
        Graphics::Gdi::{InvalidateRect, UpdateWindow, ValidateRect},
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext},
            Input::KeyboardAndMouse::*,
            WindowsAndMessaging::*,
        },
    },
    core::*,
};

use crate::geom::{minutes_at, Hit, Pt, RING_CENTER};
use crate::gfx::{Canvas, Gfx};
use crate::model::{fmt_hhmm, local_now, DeletedTask, Schedule, DAY_F};
use crate::platform::{self, outer_size};
use crate::view::{self, canvas_for, field_rect, to_design, Focus, Hot, Scene, TimeField, UiLayout};

const TIMER_CLOCK: usize = 1;
const TIMER_CARET: usize = 2;
const TIMER_SAVE: usize = 3;
const SAVE_DELAY_MS: u32 = 350;
const CLIENT_W: f32 = 980.0;
const CLIENT_H: f32 = 800.0;
const MIN_CLIENT_W: f32 = 760.0;
const MIN_CLIENT_H: f32 = 632.0;

thread_local! {
    static APP: RefCell<Option<App>> = RefCell::new(None);
}

pub struct Launch {
    pub data: PathBuf,
    pub now: Option<f32>,
    pub date: Option<(i32, u32, u32)>,
    pub screenshot: Option<PathBuf>,
}

pub fn run(launch: Launch) -> std::result::Result<(), String> {
    let mut schedule = Schedule::load_or_demo(&launch.data);
    schedule.clear_selection();
    if let Some(path) = &launch.screenshot {
        render_screenshot(path, &schedule, launch.now, launch.date)?;
        println!("screenshot {}", path.display());
        return Ok(());
    }
    unsafe { run_window(launch, schedule) }.map_err(|error| error.to_string())
}

fn render_screenshot(
    path: &Path,
    schedule: &Schedule,
    now: Option<f32>,
    date: Option<(i32, u32, u32)>,
) -> std::result::Result<(), String> {
    let mut gfx = Gfx::new().map_err(|error| error.to_string())?;
    let canvas = canvas_for(crate::geom::DESIGN_W, crate::geom::DESIGN_H);
    gfx.set_canvas(canvas);
    let now_min = now.unwrap_or_else(|| local_now().3);
    let layout = view::layout(&mut gfx, schedule, None, canvas, true, now_min);
    let date = date.unwrap_or_else(|| {
        let (year, month, day, _) = local_now();
        (year, month, day)
    });
    let displayed = schedule.displayed_task(now_min);
    let name = displayed.map(|task| task.name.clone()).unwrap_or_default();
    let start = displayed
        .map(|task| fmt_hhmm(task.start_min))
        .unwrap_or_default();
    let end = displayed
        .map(|task| fmt_hhmm(task.end_min))
        .unwrap_or_default();
    let scene = Scene {
        schedule,
        now_min,
        date,
        name_text: &name,
        start_text: &start,
        end_text: &end,
        focus: Focus::None,
        caret: 0,
        anchor: 0,
        caret_on: false,
        hover: Hot::None,
        popup: None,
        time_error: None,
        deleted_name: None,
    };
    let width = crate::geom::DESIGN_W as u32;
    let height = crate::geom::DESIGN_H as u32;
    let rgba = gfx
        .render_rgba(width, height, |gfx| view::paint(gfx, &scene, &layout))
        .map_err(|error| error.to_string())?;
    write_png(path, width, height, &rgba)
}

fn write_png(path: &Path, width: u32, height: u32, rgba: &[u8]) -> std::result::Result<(), String> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
    }
    let file = std::fs::File::create(path).map_err(|error| error.to_string())?;
    let mut encoder = png::Encoder::new(file, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|error| error.to_string())?;
    writer.write_image_data(rgba).map_err(|error| error.to_string())?;
    Ok(())
}

struct Editor {
    text: String,
    caret: usize,
    anchor: usize,
}

impl Editor {
    fn new() -> Self {
        Self {
            text: String::new(),
            caret: 0,
            anchor: 0,
        }
    }

    fn set(&mut self, text: String) {
        self.caret = text.len();
        self.anchor = self.caret;
        self.text = text;
    }

    fn range(&self) -> (usize, usize) {
        if self.caret < self.anchor {
            (self.caret, self.anchor)
        } else {
            (self.anchor, self.caret)
        }
    }

    fn delete_selection(&mut self) -> bool {
        let (start, end) = self.range();
        if start == end {
            return false;
        }
        self.text.replace_range(start..end, "");
        self.caret = start;
        self.anchor = start;
        true
    }

    fn insert(&mut self, extra: &str) {
        self.delete_selection();
        let room = 200usize.saturating_sub(self.text.chars().count());
        let extra: String = extra
            .chars()
            .filter(|ch| *ch != '\n' && *ch != '\r' && *ch != '\t')
            .take(room)
            .collect();
        self.text.insert_str(self.caret, &extra);
        self.caret += extra.len();
        self.anchor = self.caret;
    }

    fn step(&mut self, forward: bool, shift: bool) {
        self.caret = if forward {
            next_boundary(&self.text, self.caret)
        } else {
            prev_boundary(&self.text, self.caret)
        };
        if !shift {
            self.anchor = self.caret;
        }
    }

    fn jump(&mut self, end: bool, shift: bool) {
        self.caret = if end { self.text.len() } else { 0 };
        if !shift {
            self.anchor = self.caret;
        }
    }
}

struct Press {
    hot: Hot,
    start: Pt,
    moved: bool,
}

enum Drag {
    Move {
        id: u64,
        origin_start: i32,
        origin_end: i32,
        last_min: f32,
        accum: f32,
    },
    Start { id: u64 },
    End { id: u64 },
}

struct App {
    hwnd: HWND,
    gfx: Option<Gfx>,
    schedule: Schedule,
    path: PathBuf,
    dirty: bool,
    bound: Option<u64>,
    name: Editor,
    start: Editor,
    end: Editor,
    focus: Focus,
    caret_on: bool,
    hover: Hot,
    press: Option<Press>,
    drag: Option<Drag>,
    popup: Option<TimeField>,
    popup_anchor: Pt,
    toolbar_open: bool,
    time_error: Option<TimeField>,
    deleted: Option<DeletedTask>,
    notice_until: Option<Instant>,
    now_override: Option<f32>,
    date_override: Option<(i32, u32, u32)>,
}

impl Drop for App {
    fn drop(&mut self) {
        if !self.hwnd.is_invalid() { self.flush(); }
    }
}

unsafe fn run_window(launch: Launch, schedule: Schedule) -> Result<()> {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let instance: HINSTANCE = GetModuleHandleW(None)?.into();
        let class_name = w!("dayring");
        let icon = LoadIconW(Some(instance), PCWSTR::from_raw(1 as *const u16)).unwrap_or_default();
        let atom = RegisterClassW(&WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wndproc),
            hInstance: instance,
            hIcon: icon,
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            lpszClassName: class_name,
            ..Default::default()
        });
        if atom == 0 {
            return Err(Error::from_thread());
        }

        let monitor = platform::cursor_monitor();
        let dpi = (monitor.dpi * 96.0).round() as u32;
        let client_w = (CLIENT_W * monitor.dpi).round() as i32;
        let client_h = (CLIENT_H * monitor.dpi).round() as i32;
        let (width, height) = outer_size(client_w, client_h, dpi);
        let work_w = monitor.work.right - monitor.work.left;
        let work_h = monitor.work.bottom - monitor.work.top;
        let x = (monitor.work.left + (work_w - width) / 2).max(monitor.work.left);
        let y = (monitor.work.top + (work_h - height) / 2).max(monitor.work.top);
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            class_name,
            w!("dayring"),
            WS_OVERLAPPEDWINDOW,
            x,
            y,
            width,
            height,
            None,
            None,
            Some(instance),
            None,
        )?;

        let name_text = schedule.selected_task().map(|task| task.name.clone()).unwrap_or_default();
        let mut name = Editor::new();
        name.set(name_text);
        let app = App {
            hwnd,
            gfx: Some(Gfx::new()?),
            bound: schedule.selected_id(),
            schedule,
            path: launch.data,
            dirty: false,
            name,
            start: Editor::new(),
            end: Editor::new(),
            focus: Focus::None,
            caret_on: true,
            hover: Hot::None,
            press: None,
            drag: None,
            popup: None,
            popup_anchor: Pt::new(0.0, 0.0),
            toolbar_open: true,
            time_error: None,
            deleted: None,
            notice_until: None,
            now_override: launch.now,
            date_override: launch.date,
        };
        APP.with(|slot| *slot.borrow_mut() = Some(app));
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = UpdateWindow(hwnd);
        let _ = SetTimer(Some(hwnd), TIMER_CLOCK, 1000, None);

        let mut message = MSG::default();
        while GetMessageW(&mut message, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        APP.with(|slot| slot.borrow_mut().take());
        Ok(())
    }
}

fn with_app(f: impl FnOnce(&mut App) -> Option<LRESULT>) -> Option<LRESULT> {
    APP.with(|slot| {
        let mut slot = slot.try_borrow_mut().ok()?;
        let app = slot.as_mut()?;
        f(app)
    })
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match with_app(|app| app.on_message(msg, wp, lp)) {
        Some(result) => result,
        None => unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
    }
}

impl App {
    fn on_message(&mut self, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
        match msg {
            WM_ERASEBKGND => Some(LRESULT(1)),
            WM_PAINT => {
                self.paint();
                Some(LRESULT(0))
            }
            WM_SIZE => {
                self.invalidate();
                Some(LRESULT(0))
            }
            WM_DPICHANGED => {
                let rect = unsafe { &*(lp.0 as *const RECT) };
                unsafe {
                    let _ = SetWindowPos(
                        self.hwnd,
                        None,
                        rect.left,
                        rect.top,
                        rect.right - rect.left,
                        rect.bottom - rect.top,
                        SWP_NOZORDER | SWP_NOACTIVATE,
                    );
                }
                self.invalidate();
                Some(LRESULT(0))
            }
            WM_GETMINMAXINFO => {
                let info = unsafe { &mut *(lp.0 as *mut MINMAXINFO) };
                let (width, height) = self.min_track();
                info.ptMinTrackSize.x = width;
                info.ptMinTrackSize.y = height;
                Some(LRESULT(0))
            }
            WM_SETCURSOR if (lp.0 as u32 & 0xFFFF) == HTCLIENT as u32 => {
                self.apply_cursor();
                Some(LRESULT(1))
            }
            WM_MOUSEMOVE => {
                let (x, y) = lparam_point(lp);
                self.mouse_move(x, y);
                Some(LRESULT(0))
            }
            WM_LBUTTONDOWN => {
                let (x, y) = lparam_point(lp);
                self.mouse_down(x, y);
                Some(LRESULT(0))
            }
            WM_LBUTTONUP => {
                let (x, y) = lparam_point(lp);
                self.mouse_up(x, y);
                Some(LRESULT(0))
            }
            WM_CAPTURECHANGED | WM_CANCELMODE => {
                self.end_pointer_action(None);
                self.invalidate();
                None
            }
            WM_TIMER => match wp.0 {
                TIMER_CLOCK => {
                    if self.notice_until.is_some_and(|until| Instant::now() >= until) {
                        self.notice_until = None;
                    }
                    if self.dirty {
                        self.flush();
                    }
                    self.invalidate();
                    Some(LRESULT(0))
                }
                TIMER_CARET => {
                    self.caret_on = !self.caret_on;
                    self.invalidate();
                    Some(LRESULT(0))
                }
                TIMER_SAVE => {
                    unsafe {
                        let _ = KillTimer(Some(self.hwnd), TIMER_SAVE);
                    }
                    self.flush();
                    Some(LRESULT(0))
                }
                _ => None,
            },
            WM_KEYDOWN => {
                if self.key_down(wp.0 as u16) {
                    Some(LRESULT(0))
                } else {
                    None
                }
            }
            WM_CHAR => {
                self.on_char(wp.0 as u16);
                Some(LRESULT(0))
            }
            WM_SYSCHAR => Some(LRESULT(0)),
            WM_SYSCOMMAND if wp.0 & 0xFFF0 == SC_KEYMENU as usize => Some(LRESULT(0)),
            WM_IME_STARTCOMPOSITION | WM_IME_COMPOSITION => {
                self.position_ime();
                None
            }
            WM_CLOSE => {
                if !self.blur_commit() {
                    self.cancel_edit();
                }
                self.flush();
                None
            }
            WM_DESTROY => {
                self.flush();
                unsafe { PostQuitMessage(0) };
                Some(LRESULT(0))
            }
            _ => None,
        }
    }

    fn paint(&mut self) {
        let now_min = self.now_minutes();
        let (width, height) = self.client_size();
        if width == 0 || height == 0 {
            unsafe {
                let _ = ValidateRect(Some(self.hwnd), None);
            }
            return;
        }
        let canvas = canvas_for(width as f32, height as f32);
        let popup_at = self.popup.map(|field| (field, self.popup_anchor));
        let focus = self.focus;
        let (caret, anchor) = self.editor().map(|editor| (editor.caret, editor.anchor)).unwrap_or((0, 0));
        let caret_on = self.caret_on;
        let hover = self.hover;
        let date = self.today();
        let popup = self.popup;
        let displayed = self.schedule.displayed_task(now_min);
        let name = if focus == Focus::Name {
            self.name.text.clone()
        } else {
            displayed.map(|task| task.name.clone()).unwrap_or_default()
        };
        let start = if focus == Focus::Start {
            self.start.text.clone()
        } else {
            displayed.map(|task| fmt_hhmm(task.start_min)).unwrap_or_default()
        };
        let end = if focus == Focus::End {
            self.end.text.clone()
        } else {
            displayed.map(|task| fmt_hhmm(task.end_min)).unwrap_or_default()
        };
        let hwnd = self.hwnd;
        let mut gfx = self.gfx.take().expect("drawing context");
        let layout = view::layout(&mut gfx, &self.schedule, popup_at, canvas, self.tools_visible(), now_min);
        gfx.set_canvas(canvas);
        if gfx.begin_hwnd(hwnd).unwrap_or(false) {
            let scene = Scene {
                schedule: &self.schedule,
                now_min,
                date,
                name_text: &name,
                start_text: &start,
                end_text: &end,
                focus,
                caret,
                anchor,
                caret_on,
                hover,
                popup,
                time_error: self.time_error,
                deleted_name: self.deleted_name(),
            };
            view::paint(&mut gfx, &scene, &layout);
            gfx.end();
        }
        self.gfx = Some(gfx);
        unsafe {
            let _ = ValidateRect(Some(hwnd), None);
        }
    }

    fn mouse_down(&mut self, x: f32, y: f32) {
        let (hot, point, layout, _) = self.hot_at(x, y);
        if self.popup.is_some() && !matches!(hot, Hot::Hour(_) | Hot::Minute(_) | Hot::Popup) {
            // Closing a picker must not consume a click on the ring or another task.
            self.close_popup();
        }
        if !matches!(hot, Hot::Field(_) | Hot::Clock(_) | Hot::Hour(_) | Hot::Minute(_) | Hot::Popup)
            && self.focus != Focus::None {
            if !self.blur_commit() {
                self.invalidate();
                return;
            }
            self.finish_modification();
        }
        unsafe { let _ = SetCapture(self.hwnd); }
        self.press = Some(Press { hot, start: point, moved: false });
        match hot {
            Hot::Hour(hour) => self.pick_hour(hour),
            Hot::Minute(minute) => self.pick_minute(minute),
            Hot::Popup => {}
            Hot::Clock(field) => {
                if !self.ensure_selection(layout.center_task) { return; }
                if matches!(self.focus, Focus::Start | Focus::End) {
                    self.cancel_edit();
                } else if !self.blur_commit() {
                    return;
                }
                self.popup = Some(field);
                self.popup_anchor = point;
            }
            Hot::Field(focus) => self.focus_field(focus, point, &layout),
            Hot::Chip(id) => self.select_task(id, false),
            Hot::Ring(Hit::HandleStart(id)) => self.begin_drag(id, true, point),
            Hot::Ring(Hit::HandleEnd(id)) => self.begin_drag(id, false, point),
            Hot::Ring(Hit::Body(id)) => self.begin_move(id, point),
            Hot::Ring(Hit::Miss) | Hot::None => self.toolbar_open = false,
            Hot::Delete | Hot::Color(_) => {}
            Hot::Ring(Hit::Empty) | Hot::Undo => {}
        }
        self.hover = hot;
        self.invalidate();
    }

    fn mouse_move(&mut self, x: f32, y: f32) {
        let (hot, point, _, _) = self.hot_at(x, y);
        let mut moved = false;
        if let Some(press) = &mut self.press {
            if !press.moved && point.dist(press.start) > 3.0 {
                press.moved = true;
                moved = true;
            }
        }
        let dragged = self.update_drag(point);
        if dragged {
            if let Some(press) = self.press.as_mut() { press.moved = true; }
        }
        let hover_changed = hot != self.hover;
        if hover_changed {
            self.hover = hot;
        }
        if moved || dragged || hover_changed {
            self.invalidate();
        }
    }

    fn mouse_up(&mut self, x: f32, y: f32) {
        let (hot, _, _, _) = self.hot_at(x, y);
        self.end_pointer_action(Some(hot));
        unsafe {
            let _ = ReleaseCapture();
        }
        self.hover = hot;
        self.invalidate();
    }

    fn end_pointer_action(&mut self, hot: Option<Hot>) {
        let was_dragging = self.drag.take().is_some();
        if let Some(press) = self.press.take() {
            if was_dragging && press.moved {
                self.finish_modification();
            }
            if !press.moved && Some(press.hot) == hot {
                self.activate(press.hot, press.start);
            }
        }
    }

    fn activate(&mut self, hot: Hot, point: Pt) {
        match hot {
            Hot::Delete => self.delete_selected(),
            Hot::Undo => self.undo_delete(),
            Hot::Color(index) => {
                if !self.blur_commit() { return; }
                if self.schedule.set_color(index) { self.mark(); }
                self.finish_modification();
            }
            Hot::Ring(Hit::Empty) => {
                if !self.blur_commit() { return; }
                let start = minutes_at(RING_CENTER, point).round() as i32;
                self.schedule.add_task(start);
                self.sync_selection();
                self.toolbar_open = true;
                self.begin_edit(Focus::Name);
                self.name.anchor = 0;
                self.mark();
            }
            _ => {}
        }
    }

    fn begin_move(&mut self, id: u64, point: Pt) {
        if !self.blur_commit() { return; }
        self.select_task(id, false);
        let Some(task) = self.schedule.tasks().iter().find(|task| task.id == id).cloned() else {
            return;
        };
        self.drag = Some(Drag::Move {
            id,
            origin_start: task.start_min,
            origin_end: task.end_min,
            last_min: minutes_at(RING_CENTER, point),
            accum: 0.0,
        });
    }

    fn begin_drag(&mut self, id: u64, start_handle: bool, _point: Pt) {
        if !self.blur_commit() { return; }
        self.select_task(id, false);
        self.drag = Some(if start_handle {
            Drag::Start { id }
        } else {
            Drag::End { id }
        });
    }

    fn update_drag(&mut self, point: Pt) -> bool {
        let minutes = minutes_at(RING_CENTER, point);
        match self.drag.as_mut() {
            Some(Drag::Move {
                id,
                origin_start,
                origin_end,
                last_min,
                accum,
            }) => {
                *accum += shortest_step(minutes, *last_min);
                *last_min = minutes;
                let id = *id;
                let origin_start = *origin_start;
                let origin_end = *origin_end;
                let accum = *accum;
                if self.schedule.move_by(id, origin_start, origin_end, accum) {
                    self.mark();
                    true
                } else {
                    false
                }
            }
            Some(Drag::Start { id }) => {
                let id = *id;
                if self.schedule.resize_start(id, minutes) {
                    self.mark();
                    true
                } else {
                    false
                }
            }
            Some(Drag::End { id }) => {
                let id = *id;
                if self.schedule.resize_end(id, minutes) {
                    self.mark();
                    true
                } else {
                    false
                }
            }
            None => false,
        }
    }

    fn begin_edit(&mut self, focus: Focus) {
        if focus == Focus::None { return; }
        let task_id = self.schedule.displayed_task(self.now_minutes()).map(|task| task.id);
        if !self.ensure_selection(task_id) { return; }
        let text = match focus {
            Focus::Name => self.schedule.selected_task().unwrap().name.clone(),
            Focus::Start => self.saved_time(TimeField::Start),
            Focus::End => self.saved_time(TimeField::End),
            Focus::None => return,
        };
        self.focus = focus;
        self.time_error = None;
        self.editor_mut().unwrap().set(text);
        self.caret_on = true;
        self.arm_caret();
    }

    fn focus_field(&mut self, focus: Focus, point: Pt, layout: &UiLayout) {
        if !self.ensure_selection(layout.center_task) { return; }
        let shift = key_down(VK_SHIFT.0);
        let switching = self.focus != focus;
        if switching {
            if !self.blur_commit() { return; }
            self.begin_edit(focus);
        }
        let text = self.focused_text();
        let Some(rect) = field_rect(layout, focus) else { return; };
        let caret = self.editor().map(|editor| editor.caret).unwrap_or(0);
        let mut gfx = self.gfx.take().expect("drawing context");
        let (size, bold) = view::field_style(focus);
        let origin = view::field_text_x(&mut gfx, rect, focus, &text, caret);
        let index = gfx.caret_index(&text, size, bold, point.x - origin);
        self.gfx = Some(gfx);
        if let Some(editor) = self.editor_mut() {
            editor.caret = index;
            if switching || !shift { editor.anchor = index; }
        }
    }

    fn key_down(&mut self, vk: u16) -> bool {
        let ctrl = key_down(VK_CONTROL.0);
        let shift = key_down(VK_SHIFT.0);
        if vk == VK_ESCAPE.0 {
            if self.popup.is_some() {
                self.close_popup();
                self.invalidate();
                return true;
            }
            if self.focus != Focus::None {
                self.cancel_edit();
                self.finish_modification();
            }
            else { self.toolbar_open = false; }
            self.invalidate();
            return true;
        }
        if self.focus == Focus::None && self.popup.is_none() && self.drag.is_none() {
            if vk == VK_DELETE.0 {
                self.delete_selected();
                self.invalidate();
                return true;
            }
            if ctrl && vk == VK_Z.0 {
                self.undo_delete();
                self.invalidate();
                return true;
            }
        }
        if vk == VK_TAB.0 && self.schedule.displayed_task(self.now_minutes()).is_some() {
            self.popup = None;
            self.cycle_focus(shift);
            self.invalidate();
            return true;
        }
        if vk == VK_RETURN.0 && matches!(self.focus, Focus::Start | Focus::End | Focus::Name) {
            if self.blur_commit() { self.finish_modification(); }
            self.invalidate();
            return true;
        }
        if self.focus == Focus::None {
            return false;
        }
        let edited = match vk {
            key if key == VK_BACK.0 => {
                self.backspace();
                true
            }
            key if key == VK_DELETE.0 => {
                self.delete_forward();
                true
            }
            key if key == VK_LEFT.0 => {
                if let Some(editor) = self.editor_mut() {
                    editor.step(false, shift);
                }
                true
            }
            key if key == VK_RIGHT.0 => {
                if let Some(editor) = self.editor_mut() {
                    editor.step(true, shift);
                }
                true
            }
            key if key == VK_HOME.0 => {
                if let Some(editor) = self.editor_mut() {
                    editor.jump(false, shift);
                }
                true
            }
            key if key == VK_END.0 => {
                if let Some(editor) = self.editor_mut() {
                    editor.jump(true, shift);
                }
                true
            }
            key if key == VK_A.0 && ctrl => {
                if let Some(editor) = self.editor_mut() {
                    editor.anchor = 0;
                    editor.caret = editor.text.len();
                }
                true
            }
            key if key == VK_C.0 && ctrl => {
                self.copy();
                false
            }
            key if key == VK_X.0 && ctrl => {
                self.cut();
                true
            }
            key if key == VK_V.0 && ctrl => {
                self.paste();
                true
            }
            _ => false,
        };
        if edited {
            self.caret_on = true;
            self.time_error = None;
            self.invalidate();
        }
        edited || ctrl
    }

    fn on_char(&mut self, unit: u16) {
        if self.focus == Focus::None {
            return;
        }
        let Some(ch) = char::from_u32(unit as u32) else {
            return;
        };
        if ch.is_control() {
            return;
        }
        if let Some(editor) = self.editor_mut() {
            editor.insert(&ch.to_string());
        }
        self.caret_on = true;
        self.time_error = None;
        self.invalidate();
    }

    fn cycle_focus(&mut self, backward: bool) {
        // Name → Start → End → Name. Shift walks that cycle backward.
        // With nothing focused, Tab enters at the name and Shift+Tab at the end.
        let next = if backward {
            match self.focus {
                Focus::None | Focus::Name => Focus::End,
                Focus::Start => Focus::Name,
                Focus::End => Focus::Start,
            }
        } else {
            match self.focus {
                Focus::None | Focus::End => Focus::Name,
                Focus::Name => Focus::Start,
                Focus::Start => Focus::End,
            }
        };
        if self.blur_commit() { self.begin_edit(next); }
    }

    fn backspace(&mut self) {
        let Some(editor) = self.editor_mut() else {
            return;
        };
        if !editor.delete_selection() {
            let previous = prev_boundary(&editor.text, editor.caret);
            if previous != editor.caret {
                editor.text.replace_range(previous..editor.caret, "");
                editor.caret = previous;
                editor.anchor = previous;
            }
        }
    }

    fn delete_forward(&mut self) {
        let Some(editor) = self.editor_mut() else {
            return;
        };
        if !editor.delete_selection() {
            let next = next_boundary(&editor.text, editor.caret);
            if next != editor.caret {
                editor.text.replace_range(editor.caret..next, "");
                editor.anchor = editor.caret;
            }
        }
    }

    fn copy(&self) {
        let Some(editor) = self.editor() else {
            return;
        };
        let (start, end) = editor.range();
        if start != end {
            platform::clipboard_set(&editor.text[start..end]);
        }
    }

    fn cut(&mut self) {
        self.copy();
        if let Some(editor) = self.editor_mut() {
            editor.delete_selection();
        }
    }

    fn paste(&mut self) {
        let Some(text) = platform::clipboard_get() else {
            return;
        };
        if let Some(editor) = self.editor_mut() {
            editor.insert(&text);
        }
    }

    fn commit_focus(&mut self) -> bool {
        match self.focus {
            Focus::Name => {
                if self.schedule.set_name(self.name.text.clone()) { self.mark(); }
            }
            Focus::Start | Focus::End => {
                let field = if self.focus == Focus::Start { TimeField::Start } else { TimeField::End };
                let text = self.focused_text();
                let Some(minutes) = crate::model::parse_hhmm(&text) else {
                    self.time_error = Some(field);
                    self.invalidate();
                    return false;
                };
                let changed = match field {
                    TimeField::Start => self.schedule.set_start(minutes),
                    TimeField::End => self.schedule.set_end(minutes),
                };
                if changed { self.mark(); }
            }
            Focus::None => {}
        }
        self.time_error = None;
        true
    }

    fn blur_commit(&mut self) -> bool {
        if !self.commit_focus() { return false; }
        self.focus = Focus::None;
        self.kill_caret();
        true
    }

    fn cancel_edit(&mut self) {
        self.focus = Focus::None;
        self.time_error = None;
        self.name.set(self.schedule.selected_task().map(|task| task.name.clone()).unwrap_or_default());
        self.start.set(self.saved_time(TimeField::Start));
        self.end.set(self.saved_time(TimeField::End));
        self.kill_caret();
    }

    fn pick_hour(&mut self, hour: i32) {
        let Some(field) = self.popup else {
            return;
        };
        let Some(current) = self.saved_minutes(field) else {
            return;
        };
        self.apply_popup(field, hour * 60 + current % 60);
    }

    fn pick_minute(&mut self, minute: i32) {
        let Some(field) = self.popup else {
            return;
        };
        let Some(current) = self.saved_minutes(field) else {
            return;
        };
        self.apply_popup(field, (current / 60) * 60 + minute);
    }

    fn apply_popup(&mut self, field: TimeField, minutes: i32) {
        let changed = match field {
            TimeField::Start => self.schedule.set_start(minutes),
            TimeField::End => self.schedule.set_end(minutes),
        };
        self.time_error = None;
        if matches!(self.focus, Focus::Start | Focus::End) {
            self.focus = Focus::None;
            self.kill_caret();
        }
        if changed {
            self.mark();
        }
    }

    fn select_task(&mut self, id: u64, force_mark: bool) {
        let changed = self.schedule.selected_id() != Some(id);
        self.schedule.select(id);
        self.sync_selection();
        self.toolbar_open = true;
        if changed || force_mark {
            self.mark();
        }
    }

    fn delete_selected(&mut self) {
        if !self.blur_commit() { return; }
        self.popup = None;
        if let Some(deleted) = self.schedule.delete_selected() {
            self.deleted = Some(deleted);
            self.notice_until = Some(Instant::now() + Duration::from_secs(5));
            self.mark();
            self.finish_modification();
        }
    }

    fn undo_delete(&mut self) {
        if !self.blur_commit() { return; }
        self.popup = None;
        if let Some(deleted) = self.deleted.take() {
            if self.schedule.restore_deleted(deleted) {
                self.mark();
                self.finish_modification();
            }
        }
        self.notice_until = None;
    }

    fn deleted_name(&self) -> Option<&str> {
        self.notice_until.filter(|until| Instant::now() < *until)?;
        self.deleted.as_ref().map(|deleted| {
            if deleted.task.name.is_empty() { "未命名" } else { deleted.task.name.as_str() }
        })
    }

    fn tools_visible(&self) -> bool {
        self.toolbar_open && self.drag.is_none() && self.focus == Focus::None && self.popup.is_none()
    }

    fn ensure_selection(&mut self, task_id: Option<u64>) -> bool {
        if self.schedule.selected_id().is_none() {
            let Some(id) = task_id else { return false; };
            self.select_task(id, false);
        }
        self.schedule.selected_task().is_some()
    }

    fn finish_modification(&mut self) {
        self.popup = None;
        self.drag = None;
        self.toolbar_open = false;
        self.schedule.clear_selection();
        self.sync_selection();
    }

    fn close_popup(&mut self) {
        if self.popup.take().is_some() { self.finish_modification(); }
    }

    fn sync_selection(&mut self) {
        let id = self.schedule.selected_id();
        if self.bound == id {
            return;
        }
        self.bound = id;
        self.focus = Focus::None;
        self.popup = None;
        self.time_error = None;
        self.kill_caret();
        let name = self.schedule.selected_task().map(|task| task.name.clone()).unwrap_or_default();
        self.name.set(name);
    }

    fn saved_time(&self, field: TimeField) -> String {
        self.saved_minutes(field).map(fmt_hhmm).unwrap_or_default()
    }

    fn saved_minutes(&self, field: TimeField) -> Option<i32> {
        let task = self.schedule.selected_task()?;
        Some(match field {
            TimeField::Start => task.start_min,
            TimeField::End => task.end_min,
        })
    }

    fn focused_text(&self) -> String {
        match self.focus {
            Focus::Name => self.name.text.clone(),
            Focus::Start => self.start.text.clone(),
            Focus::End => self.end.text.clone(),
            Focus::None => String::new(),
        }
    }

    fn editor(&self) -> Option<&Editor> {
        match self.focus {
            Focus::Name => Some(&self.name),
            Focus::Start => Some(&self.start),
            Focus::End => Some(&self.end),
            Focus::None => None,
        }
    }

    fn editor_mut(&mut self) -> Option<&mut Editor> {
        match self.focus {
            Focus::Name => Some(&mut self.name),
            Focus::Start => Some(&mut self.start),
            Focus::End => Some(&mut self.end),
            Focus::None => None,
        }
    }

    fn hot_at(&mut self, x: f32, y: f32) -> (Hot, Pt, UiLayout, Canvas) {
        let canvas = self.canvas();
        let point = to_design(canvas, x, y);
        let popup = self.popup.map(|field| (field, self.popup_anchor));
        let mut gfx = self.gfx.take().expect("drawing context");
        let layout = view::layout(&mut gfx, &self.schedule, popup, canvas, self.tools_visible(), self.now_minutes());
        let hot = view::hit(&layout, &self.schedule, point, self.deleted_name().is_some());
        self.gfx = Some(gfx);
        (hot, point, layout, canvas)
    }

    fn position_ime(&mut self) {
        let focus = self.focus;
        if focus == Focus::None {
            return;
        }
        let text = self.focused_text();
        let caret = self.editor().map(|editor| editor.caret).unwrap_or(0);
        let canvas = self.canvas();
        let popup = self.popup.map(|field| (field, self.popup_anchor));
        let mut gfx = self.gfx.take().expect("drawing context");
        let layout = view::layout(&mut gfx, &self.schedule, popup, canvas, self.tools_visible(), self.now_minutes());
        let Some(rect) = field_rect(&layout, focus) else {
            self.gfx = Some(gfx);
            return;
        };
        let caret = caret.min(text.len());
        let (size, bold) = view::field_style(focus);
        let origin = view::field_text_x(&mut gfx, rect, focus, &text, caret);
        let width = gfx.measure(&text[..caret], size, bold);
        self.gfx = Some(gfx);
        let x = canvas.origin.0 + (origin + width) * canvas.k;
        let y = canvas.origin.1 + (rect.y + rect.h * 0.75) * canvas.k;
        platform::ime_follow(self.hwnd, x.round() as i32, y.round() as i32);
    }

    fn canvas(&self) -> Canvas {
        let (width, height) = self.client_size();
        canvas_for(width as f32, height as f32)
    }

    fn client_size(&self) -> (u32, u32) {
        unsafe {
            let mut rect = RECT::default();
            let _ = GetClientRect(self.hwnd, &mut rect);
            (rect.right.max(0) as u32, rect.bottom.max(0) as u32)
        }
    }

    fn min_track(&self) -> (i32, i32) {
        let dpi = platform::window_dpi(self.hwnd);
        let client_w = (MIN_CLIENT_W * dpi).round() as i32;
        let client_h = (MIN_CLIENT_H * dpi).round() as i32;
        outer_size(client_w, client_h, (dpi * 96.0).round() as u32)
    }

    fn now_minutes(&self) -> f32 {
        self.now_override.unwrap_or_else(|| local_now().3)
    }

    fn today(&self) -> (i32, u32, u32) {
        if let Some(date) = self.date_override {
            date
        } else {
            let (year, month, day, _) = local_now();
            (year, month, day)
        }
    }

    fn mark(&mut self) {
        self.dirty = true;
        if self.hwnd.is_invalid() { return; }
        unsafe {
            let _ = SetTimer(Some(self.hwnd), TIMER_SAVE, SAVE_DELAY_MS, None);
        }
    }

    fn flush(&mut self) {
        if !self.dirty {
            return;
        }
        if let Err(error) = self.schedule.save(&self.path) {
            eprintln!("保存日程失败: {error}");
            return;
        }
        self.dirty = false;
    }

    fn invalidate(&self) {
        if self.hwnd.is_invalid() { return; }
        unsafe {
            let _ = InvalidateRect(Some(self.hwnd), None, false);
        }
    }

    fn arm_caret(&self) {
        if self.hwnd.is_invalid() { return; }
        unsafe {
            let _ = SetTimer(Some(self.hwnd), TIMER_CARET, 530, None);
        }
    }

    fn kill_caret(&self) {
        if self.hwnd.is_invalid() { return; }
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_CARET);
        }
    }

    fn apply_cursor(&self) {
        unsafe {
            if let Ok(cursor) = LoadCursorW(None, self.cursor_id()) {
                SetCursor(Some(cursor));
            }
        }
    }

    fn cursor_id(&self) -> PCWSTR {
        if let Some(drag) = &self.drag {
            return match drag {
                Drag::Move { .. } => IDC_SIZEALL,
                Drag::Start { .. } | Drag::End { .. } => IDC_HAND,
            };
        }
        match self.hover {
            Hot::Field(_) => IDC_IBEAM,
            Hot::Ring(Hit::Empty) => IDC_CROSS,
            Hot::Ring(Hit::Body(_)) => IDC_SIZEALL,
            Hot::Ring(Hit::HandleStart(_) | Hit::HandleEnd(_)) => IDC_HAND,
            Hot::None | Hot::Ring(Hit::Miss) | Hot::Popup => IDC_ARROW,
            _ => IDC_HAND,
        }
    }
}

fn key_down(vk: u16) -> bool {
    unsafe { GetKeyState(vk as i32) < 0 }
}

fn lparam_point(lp: LPARAM) -> (f32, f32) {
    let packed = lp.0 as u32;
    let x = (packed & 0xFFFF) as i16 as f32;
    let y = (packed >> 16) as i16 as f32;
    (x, y)
}

fn shortest_step(current: f32, last: f32) -> f32 {
    let mut step = (current - last).rem_euclid(DAY_F);
    if step > DAY_F * 0.5 {
        step -= DAY_F;
    }
    step
}

fn prev_boundary(text: &str, index: usize) -> usize {
    if index == 0 {
        return 0;
    }
    let mut next = index - 1;
    while next > 0 && !text.is_char_boundary(next) {
        next -= 1;
    }
    next
}

fn next_boundary(text: &str, index: usize) -> usize {
    if index >= text.len() {
        return text.len();
    }
    let mut next = index + 1;
    while next < text.len() && !text.is_char_boundary(next) {
        next += 1;
    }
    next
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::{point_at, RING_RADIUS};

    fn app() -> App {
        let mut schedule = Schedule::demo();
        schedule.clear_selection();
        App {
            hwnd: HWND::default(), gfx: None, schedule,
            path: PathBuf::new(), dirty: false, bound: None,
            name: Editor::new(), start: Editor::new(), end: Editor::new(),
            focus: Focus::None, caret_on: false, hover: Hot::None,
            press: None, drag: None, popup: None,
            popup_anchor: Pt::new(0.0, 0.0), toolbar_open: false,
            time_error: None, deleted: None, notice_until: None,
            now_override: Some(590.0), date_override: None,
        }
    }

    fn displayed(app: &App) -> Option<u64> {
        app.schedule.displayed_task(app.now_minutes()).map(|task| task.id)
    }

    fn assert_following(app: &App, id: Option<u64>) {
        assert_eq!(app.schedule.selected_id(), None);
        assert_eq!(app.focus, Focus::None);
        assert!(app.popup.is_none() && app.drag.is_none());
        assert!(!app.tools_visible());
        assert_eq!(displayed(app), id);
    }

    #[test]
    fn current_display_changes_without_selecting_or_saving() {
        let mut app = app();
        for (time, id) in [(590.0, Some(2)), (700.0, None), (730.0, Some(3))] {
            app.now_override = Some(time);
            assert_following(&app, id);
        }
        assert!(!app.dirty);
        app.key_down(VK_DELETE.0);
        assert_eq!(app.schedule.tasks().len(), 5);
        assert!(!app.dirty);
    }

    #[test]
    fn manual_selection_survives_clock_changes_until_name_is_confirmed() {
        let mut app = app();
        app.select_task(5, false);
        app.now_override = Some(730.0);
        assert_eq!(displayed(&app), Some(5));
        app.begin_edit(Focus::Name);
        app.name.set("新的名称".into());
        app.key_down(VK_RETURN.0);
        assert_eq!(app.schedule.tasks()[4].name, "新的名称");
        assert_following(&app, Some(3));
    }

    #[test]
    fn fields_share_one_selection_and_invalid_time_does_not_finish() {
        let mut app = app();
        app.select_task(5, false);
        app.begin_edit(Focus::Name);
        app.name.set("修改后的阅读".into());
        app.cycle_focus(false);
        assert_eq!(app.focus, Focus::Start);
        assert_eq!(app.schedule.selected_id(), Some(5));
        assert_eq!(app.schedule.tasks()[4].name, "修改后的阅读");
        app.start.set("99:99".into());
        app.key_down(VK_RETURN.0);
        assert_eq!(app.focus, Focus::Start);
        assert_eq!(app.time_error, Some(TimeField::Start));
        assert_eq!(app.schedule.selected_id(), Some(5));
        app.start.set("20:15".into());
        app.key_down(VK_RETURN.0);
        assert_eq!(app.schedule.tasks()[4].start_min, 1215);
        assert_following(&app, Some(2));
    }

    #[test]
    fn escape_discards_the_field_draft_and_returns_to_current() {
        let mut app = app();
        app.select_task(5, false);
        app.begin_edit(Focus::Name);
        app.name.set("不要保存".into());
        app.key_down(VK_ESCAPE.0);
        assert_eq!(app.schedule.tasks()[4].name, "阅读");
        assert_following(&app, Some(2));
    }

    #[test]
    fn editing_current_task_pins_it_even_when_clock_crosses_its_end() {
        let mut app = app();
        app.begin_edit(Focus::Name);
        assert_eq!(app.schedule.selected_id(), Some(2));
        app.now_override = Some(700.0);
        assert_eq!(displayed(&app), Some(2));
        app.key_down(VK_RETURN.0);
        assert_following(&app, None);
    }

    #[test]
    fn unchanged_confirmed_task_can_become_the_current_display() {
        let mut app = app();
        app.begin_edit(Focus::Name);
        app.key_down(VK_RETURN.0);
        assert_following(&app, Some(2));
    }

    #[test]
    fn picker_keeps_target_for_hour_and_minute_then_finishes_on_close() {
        for escape in [false, true] {
            let mut app = app();
            app.select_task(5, false);
            app.popup = Some(TimeField::Start);
            app.pick_hour(19);
            app.pick_minute(45);
            assert_eq!(app.schedule.selected_id(), Some(5));
            assert_eq!(app.schedule.tasks()[4].start_min, 1185);
            assert_eq!(app.popup, Some(TimeField::Start));
            if escape { app.key_down(VK_ESCAPE.0); }
            else { app.close_popup(); }
            assert_following(&app, Some(2));
        }
    }

    #[test]
    fn closing_picker_can_repin_its_displayed_target_for_the_next_field() {
        let mut app = app();
        app.select_task(5, false);
        app.popup = Some(TimeField::Start);
        let target = displayed(&app);
        app.close_popup();
        assert_eq!(displayed(&app), Some(2));
        assert!(app.ensure_selection(target));
        app.begin_edit(Focus::End);
        assert_eq!(app.schedule.selected_id(), Some(5));
        assert_eq!(app.end.text, "22:00");
    }

    #[test]
    fn ring_click_selects_but_completed_move_or_resize_returns_to_current() {
        for resize in [false, true] {
            for moved in [false, true] {
                let mut app = app();
                let point = point_at(RING_CENTER, RING_RADIUS, 1200.0);
                let hot = Hot::Ring(Hit::Body(5));
                if resize { app.begin_drag(5, true, point); }
                else { app.begin_move(5, point); }
                app.press = Some(Press { hot, start: point, moved });
                if moved {
                    assert!(app.update_drag(point_at(RING_CENTER, RING_RADIUS, 1230.0)));
                }
                app.end_pointer_action(Some(hot));
                if moved { assert_following(&app, Some(2)); }
                else { assert_eq!(app.schedule.selected_id(), Some(5)); }
            }
        }
    }

    #[test]
    fn interrupted_drag_finishes_applied_changes() {
        let mut app = app();
        let point = point_at(RING_CENTER, RING_RADIUS, 1200.0);
        app.begin_move(5, point);
        app.press = Some(Press { hot: Hot::Ring(Hit::Body(5)), start: point, moved: true });
        app.update_drag(point_at(RING_CENTER, RING_RADIUS, 1230.0));
        app.end_pointer_action(None);
        assert_following(&app, Some(2));
        assert_eq!(app.schedule.tasks()[4].start_min, 1230);
    }

    #[test]
    fn color_delete_and_undo_all_finish_without_selecting_a_fallback_task() {
        let mut app = app();
        app.select_task(5, false);
        app.activate(Hot::Color(1), RING_CENTER);
        assert_eq!(app.schedule.tasks()[4].color, 1);
        assert_following(&app, Some(2));
        app.select_task(2, false);
        let before = app.schedule.tasks().to_vec();
        app.delete_selected();
        assert_eq!(app.schedule.tasks().len(), 4);
        assert_following(&app, None);
        app.undo_delete();
        assert_eq!(app.schedule.tasks(), before);
        assert_following(&app, Some(2));
    }

    #[test]
    fn new_task_stays_selected_while_naming_then_returns_to_current() {
        let mut app = app();
        app.activate(Hot::Ring(Hit::Empty), point_at(RING_CENTER, RING_RADIUS, 1110.0));
        assert_eq!(app.focus, Focus::Name);
        assert_eq!(app.schedule.selected_id(), Some(6));
        app.name.set("新任务".into());
        app.key_down(VK_RETURN.0);
        assert_eq!(app.schedule.tasks()[5].name, "新任务");
        assert_following(&app, Some(2));
    }
}
