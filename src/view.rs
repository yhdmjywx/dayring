//! Design-pixel layout and paint. One unit here is one pixel of the 1020×1142 reference.

use crate::geom::{hit_test, point_at, Hit, Pt, DESIGN_H, DESIGN_W, HANDLE_R, RING_CENTER, RING_RADIUS, RING_STROKE};
use crate::gfx::{Align, Canvas, Color, Gfx};
use crate::model::{duration, fmt_hhmm, format_date, Schedule, Task, DAY, DAY_F};

const TEXT: Color = Color::rgb(0x0D0D0D);
const MUTED: Color = Color::rgb(0x666666);
const LINE: Color = Color::rgb(0xE5E5E5);
const RED: Color = Color::rgb(0xD55F57);
const WHITE: Color = Color::rgb(0xFFFFFF);
const SWATCH: [u32; 5] = [0x438778, 0xC38C54, 0x748ABD, 0xB184AC, 0x87918B];

const POP_W: f32 = 292.0;
const POP_H: f32 = 204.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Focus {
    None,
    Name,
    Start,
    End,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TimeField {
    Start,
    End,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Hot {
    None,
    Ring(Hit),
    Chip(u64),
    Add,
    Field(Focus),
    Clock(TimeField),
    Color(usize),
    Next,
    Delete,
    Hour(i32),
    Minute(i32),
    Popup,
}

#[derive(Clone, Copy, Debug)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    pub fn contains(self, point: Pt) -> bool {
        point.x >= self.x && point.y >= self.y && point.x < self.x + self.w && point.y < self.y + self.h
    }

    pub fn right(self) -> f32 {
        self.x + self.w
    }

    pub fn center(self) -> Pt {
        Pt::new(self.x + self.w * 0.5, self.y + self.h * 0.5)
    }
}

pub struct UiLayout {
    pub chips: Vec<(u64, Rect)>,
    pub add: Rect,
    pub name: Rect,
    pub start: Rect,
    pub end: Rect,
    pub clock_start: Rect,
    pub clock_end: Rect,
    pub colors: [Rect; 5],
    pub next: Rect,
    pub delete: Rect,
    pub popup: Option<PopupLayout>,
}

pub struct PopupLayout {
    pub panel: Rect,
    pub hours: [Rect; 24],
    pub minutes: [Rect; 4],
}

pub struct Scene<'a> {
    pub schedule: &'a Schedule,
    pub now_min: f32,
    pub date: (i32, u32, u32),
    pub name_text: &'a str,
    pub start_text: &'a str,
    pub end_text: &'a str,
    pub focus: Focus,
    pub caret: usize,
    pub anchor: usize,
    pub caret_on: bool,
    pub hover: Hot,
    pub popup: Option<TimeField>,
}

pub fn canvas_for(client_w: f32, client_h: f32) -> Canvas {
    let k = (client_w / DESIGN_W).min(client_h / DESIGN_H).max(0.01);
    Canvas {
        k,
        origin: ((client_w - DESIGN_W * k) * 0.5, (client_h - DESIGN_H * k) * 0.5),
    }
}

pub fn to_design(canvas: Canvas, x: f32, y: f32) -> Pt {
    Pt::new((x - canvas.origin.0) / canvas.k, (y - canvas.origin.1) / canvas.k)
}

pub fn layout(
    gfx: &mut Gfx,
    schedule: &Schedule,
    popup: Option<(TimeField, Pt)>,
    canvas: Canvas,
) -> UiLayout {
    let tasks = schedule.tasks_by_start();
    let mut widths = Vec::with_capacity(tasks.len());
    for task in &tasks {
        widths.push(chip_width(gfx, task));
    }
    let gap = 18.0;
    let total = widths.iter().sum::<f32>() + gap * widths.len().saturating_sub(1) as f32;
    let mut x = 494.5 - total * 0.5;
    let mut chips = Vec::with_capacity(tasks.len());
    for (task, width) in tasks.iter().zip(widths) {
        chips.push((task.id, Rect::new(x, 757.0 - 21.0, width, 42.0)));
        x += width + gap;
    }

    let name = Rect::new(115.0, 986.0, 353.0, 56.0);
    let start = Rect::new(505.0, 986.0, 166.0, 56.0);
    let end = Rect::new(708.0, 986.0, 166.0, 56.0);
    let colors = [173.0, 220.0, 267.0, 314.0, 362.0].map(|cx| {
        Rect::new(cx - 18.0, 1100.0 - 18.0, 36.0, 36.0)
    });
    let delete_w = gfx.measure("删除安排", 15.0, false).max(48.0);
    UiLayout {
        chips,
        add: Rect::new(803.0, 886.0, 70.0, 50.0),
        name,
        start,
        end,
        clock_start: clock_rect(start),
        clock_end: clock_rect(end),
        colors,
        next: Rect::new(494.0 - 20.0, 1104.0 - 20.0, 40.0, 40.0),
        delete: Rect::new(874.0 - delete_w, 1100.0 - 16.0, delete_w, 32.0),
        popup: popup.map(|(field, anchor)| popup_layout(field, anchor, canvas)),
    }
}

pub fn hit(layout: &UiLayout, schedule: &Schedule, point: Pt) -> Hot {
    if let Some(popup) = &layout.popup {
        for (hour, rect) in popup.hours.iter().enumerate() {
            if rect.contains(point) {
                return Hot::Hour(hour as i32);
            }
        }
        for (index, rect) in popup.minutes.iter().enumerate() {
            if rect.contains(point) {
                return Hot::Minute([0, 15, 30, 45][index]);
            }
        }
        if popup.panel.contains(point) {
            return Hot::Popup;
        }
    }
    if layout.clock_start.contains(point) {
        return Hot::Clock(TimeField::Start);
    }
    if layout.clock_end.contains(point) {
        return Hot::Clock(TimeField::End);
    }
    if layout.add.contains(point) {
        return Hot::Add;
    }
    if layout.delete.contains(point) {
        return Hot::Delete;
    }
    if layout.next.contains(point) {
        return Hot::Next;
    }
    for (index, rect) in layout.colors.iter().enumerate() {
        if rect.contains(point) {
            return Hot::Color(index);
        }
    }
    for (id, rect) in &layout.chips {
        if rect.contains(point) {
            return Hot::Chip(*id);
        }
    }
    if layout.name.contains(point) {
        return Hot::Field(Focus::Name);
    }
    if layout.start.contains(point) {
        return Hot::Field(Focus::Start);
    }
    if layout.end.contains(point) {
        return Hot::Field(Focus::End);
    }
    Hot::Ring(hit_test(schedule.tasks(), schedule.selected_id(), point))
}

pub fn paint(gfx: &mut Gfx, scene: &Scene<'_>, layout: &UiLayout) {
    paint_header(gfx, scene);
    paint_ring(gfx, scene);
    paint_legend(gfx, scene, layout);
    text_center(gfx, "点击空白圆环添加 · 拖动色段移动 · 拖动白点调整时长", 494.5, 811.0, 14.5, false, MUTED);
    gfx.line(96.0, 858.0, 894.0, 858.0, 1.0, LINE);
    paint_editor(gfx, scene, layout);
    if let Some(popup) = &layout.popup {
        paint_popup(gfx, scene, popup);
    }
}

pub fn field_rect(layout: &UiLayout, focus: Focus) -> Option<Rect> {
    match focus {
        Focus::Name => Some(layout.name),
        Focus::Start => Some(layout.start),
        Focus::End => Some(layout.end),
        Focus::None => None,
    }
}

fn paint_header(gfx: &mut Gfx, scene: &Scene<'_>) {
    let (year, month, day) = scene.date;
    text_left(gfx, "一天的形状", 129.0, 63.0, 26.0, true, TEXT);
    text_left(gfx, &format_date(year, month, day), 131.0, 98.0, 15.0, false, MUTED);
    text_right(gfx, "24h · 循环", 846.0, 66.0, 15.0, false, MUTED);
}

fn paint_ring(gfx: &mut Gfx, scene: &Scene<'_>) {
    for hour in 0..24 {
        let minutes = hour as f32 * 60.0;
        let major = hour % 3 == 0;
        let (inner, outer, width, color) = if major {
            (248.0, 266.0, 1.7, Color::rgb(0xA0A0A0))
        } else {
            (252.0, 261.0, 1.15, Color::rgb(0xD4D4D4))
        };
        let a = point_at(RING_CENTER, inner, minutes);
        let b = point_at(RING_CENTER, outer, minutes);
        gfx.line(a.x, a.y, b.x, b.y, width, color);
        if major {
            let label = point_at(RING_CENTER, 292.0, minutes);
            text_center(gfx, &format!("{hour:02}"), label.x, label.y, 15.0, false, MUTED);
        }
    }

    let selected = scene.schedule.selected_id();
    for task in scene.schedule.tasks() {
        if Some(task.id) != selected {
            paint_arc(gfx, task, false);
        }
    }
    if let Some(task) = scene.schedule.selected_task() {
        paint_arc(gfx, task, true);
        for minutes in [task.start_min, task.end_min] {
            let point = point_at(RING_CENTER, RING_RADIUS, minutes as f32);
            gfx.circle(point.x, point.y, HANDLE_R, WHITE);
            gfx.circle_stroke(point.x, point.y, HANDLE_R, 1.3, Color::rgb(0xD8D8D8));
        }
    }

    let now = scene.now_min;
    let inner = point_at(RING_CENTER, RING_RADIUS - RING_STROKE * 0.48, now);
    let outer = point_at(RING_CENTER, RING_RADIUS + RING_STROKE * 0.52, now);
    gfx.line(inner.x, inner.y, outer.x, outer.y, 2.0, RED);
    gfx.circle(outer.x, outer.y, 4.6, RED);
    paint_center(gfx, scene);
}

fn paint_arc(gfx: &mut Gfx, task: &Task, selected: bool) {
    let from = point_at(RING_CENTER, RING_RADIUS, task.start_min as f32);
    let to = point_at(RING_CENTER, RING_RADIUS, task.end_min as f32);
    let large = duration(task.start_min, task.end_min) > DAY / 2;
    if selected {
        gfx.arc(
            (from.x, from.y),
            (to.x, to.y),
            RING_RADIUS,
            RING_STROKE + 18.0,
            halo(task.color),
            large,
        );
    }
    gfx.arc(
        (from.x, from.y),
        (to.x, to.y),
        RING_RADIUS,
        RING_STROKE,
        arc_fill(task.color, selected),
        large,
    );
}

fn paint_center(gfx: &mut Gfx, scene: &Scene<'_>) {
    text_center(gfx, "当前选中", RING_CENTER.x, 349.0, 14.0, false, MUTED);
    let name = scene.schedule.selected_task().map(|task| task.name.as_str()).unwrap_or("");
    let name = if name.is_empty() {
        if scene.schedule.selected_task().is_some() {
            "未命名"
        } else {
            "无安排"
        }
    } else {
        name
    };
    gfx.text(name, RING_CENTER.x - 170.0, 388.0 - 24.0, 340.0, 48.0, 36.0, true, TEXT, Align::Center, true);
    let range = scene
        .schedule
        .selected_task()
        .map(|task| format!("{}  —  {}", fmt_hhmm(task.start_min), fmt_hhmm(task.end_min)))
        .unwrap_or_else(|| "—".to_string());
    text_center(gfx, &range, RING_CENTER.x, 430.0, 16.0, false, MUTED);
    gfx.line(RING_CENTER.x - 16.0, 462.0, RING_CENTER.x + 16.0, 462.0, 1.0, Color::rgb(0xE4E4E4));
    let now = (scene.now_min.rem_euclid(DAY_F)) as i32;
    text_center(gfx, &format!("现在 {}", fmt_hhmm(now)), RING_CENTER.x, 494.0, 15.0, false, MUTED);
}

fn paint_legend(gfx: &mut Gfx, scene: &Scene<'_>, layout: &UiLayout) {
    let selected = scene.schedule.selected_id();
    for (id, rect) in &layout.chips {
        let Some(task) = scene.schedule.tasks().iter().find(|task| task.id == *id) else {
            continue;
        };
        if Some(task.id) == selected {
            gfx.fill(rect.x, rect.y, rect.w, rect.h, 16.0, WHITE);
            gfx.stroke(rect.x, rect.y, rect.w, rect.h, 16.0, 1.0, LINE);
        }
        let dot = Pt::new(rect.x + 16.0, rect.center().y);
        gfx.circle(dot.x, dot.y, 5.5, Color::rgb(SWATCH[task.color % SWATCH.len()]));
        text_left(
            gfx,
            &format!("{}  {}", task.name, fmt_hhmm(task.start_min)),
            dot.x + 12.0,
            rect.center().y,
            14.5,
            false,
            MUTED,
        );
    }
}

fn paint_editor(gfx: &mut Gfx, scene: &Scene<'_>, layout: &UiLayout) {
    text_left(gfx, "编辑安排", 106.0, 911.0, 20.0, true, TEXT);
    let add_border = if scene.hover == Hot::Add {
        Color::rgb(0xCFCFCF)
    } else {
        LINE
    };
    gfx.fill(layout.add.x, layout.add.y, layout.add.w, layout.add.h, 14.0, WHITE);
    gfx.stroke(layout.add.x, layout.add.y, layout.add.w, layout.add.h, 14.0, 1.0, add_border);
    let add_center = layout.add.center();
    text_center(gfx, "+ 新增", add_center.x, add_center.y, 15.0, false, TEXT);

    text_left(gfx, "任务名称", 107.0, 970.0, 14.0, false, MUTED);
    text_left(gfx, "开始", 505.0, 970.0, 14.0, false, MUTED);
    text_left(gfx, "结束", 708.0, 970.0, 14.0, false, MUTED);
    for rect in [layout.name, layout.start, layout.end] {
        gfx.fill(rect.x, rect.y, rect.w, rect.h, 14.0, WHITE);
        gfx.stroke(rect.x, rect.y, rect.w, rect.h, 14.0, 1.0, LINE);
    }
    paint_field(gfx, layout.name, scene.name_text, scene, Focus::Name, false);
    paint_field(gfx, layout.start, scene.start_text, scene, Focus::Start, true);
    paint_field(gfx, layout.end, scene.end_text, scene, Focus::End, true);
    paint_clock(gfx, layout.clock_start.center(), 7.2);
    paint_clock(gfx, layout.clock_end.center(), 7.2);

    text_left(gfx, "颜色", 107.0, 1100.0, 14.5, false, MUTED);
    let selected_color = scene.schedule.selected_task().map(|task| task.color);
    for (index, rect) in layout.colors.iter().enumerate() {
        let center = rect.center();
        if Some(index) == selected_color {
            gfx.circle_stroke(center.x, center.y, 18.5, 1.6, Color::rgb(0x1A1A1A));
        }
        gfx.circle(center.x, center.y, 14.5, Color::rgb(SWATCH[index]));
    }
    let next = layout.next.center();
    gfx.circle_stroke(next.x, next.y, 20.0, 1.15, LINE);
    paint_chevron(gfx, next);
    let delete_color = if scene.hover == Hot::Delete { TEXT } else { MUTED };
    text_right(gfx, "删除安排", 874.0, 1100.0, 15.0, false, delete_color);
}

fn paint_field(gfx: &mut Gfx, rect: Rect, text: &str, scene: &Scene<'_>, focus: Focus, reserve_clock: bool) {
    let focused = scene.focus == focus;
    let reserve = if reserve_clock { 36.0 } else { 12.0 };
    let inner_w = (rect.w - 16.0 - reserve).max(8.0);
    let (caret, anchor) = if focused {
        (scene.caret.min(text.len()), scene.anchor.min(text.len()))
    } else {
        (0, 0)
    };
    let mut shift = 0.0;
    if focused {
        let caret_x = gfx.measure(&text[..caret], 18.0, false);
        if caret_x > inner_w - 8.0 {
            shift = caret_x - (inner_w - 8.0);
        }
    }
    gfx.clip(rect.x + 8.0, rect.y + 6.0, inner_w + 8.0, rect.h - 12.0);
    if focused && caret != anchor {
        let (start, end) = if caret < anchor { (caret, anchor) } else { (anchor, caret) };
        let x0 = rect.x + 16.0 - shift + gfx.measure(&text[..start], 18.0, false);
        let x1 = rect.x + 16.0 - shift + gfx.measure(&text[..end], 18.0, false);
        gfx.fill(x0, rect.y + 16.0, (x1 - x0).max(1.0), rect.h - 32.0, 3.0, Color::rgb(0xE8E8E8));
    }
    gfx.text(text, rect.x + 16.0 - shift, rect.y, 4_000.0, rect.h, 18.0, false, TEXT, Align::Left, false);
    if focused && scene.caret_on {
        let x = rect.x + 16.0 - shift + gfx.measure(&text[..caret], 18.0, false);
        gfx.line(x, rect.y + 16.0, x, rect.y + rect.h - 16.0, 1.15, TEXT);
    }
    gfx.unclip();
}

fn paint_popup(gfx: &mut Gfx, scene: &Scene<'_>, popup: &PopupLayout) {
    let Some(field) = scene.popup else {
        return;
    };
    let panel = popup.panel;
    gfx.fill(panel.x, panel.y, panel.w, panel.h, 12.0, WHITE);
    gfx.stroke(panel.x, panel.y, panel.w, panel.h, 12.0, 1.0, LINE);
    let label = match field {
        TimeField::Start => "开始时间",
        TimeField::End => "结束时间",
    };
    text_left(gfx, label, panel.x + 12.0, panel.y + 18.0, 13.0, false, MUTED);
    let current = scene.schedule.selected_task().map(|task| match field {
        TimeField::Start => task.start_min,
        TimeField::End => task.end_min,
    });
    let Some(current) = current else {
        return;
    };
    for (hour, rect) in popup.hours.iter().enumerate() {
        let on = current / 60 == hour as i32;
        paint_cell(gfx, *rect, &format!("{hour:02}"), on);
    }
    for (index, rect) in popup.minutes.iter().enumerate() {
        let minute = [0, 15, 30, 45][index];
        let on = current % 60 == minute;
        paint_cell(gfx, *rect, &format!("{minute:02}"), on);
    }
}

fn paint_cell(gfx: &mut Gfx, rect: Rect, text: &str, on: bool) {
    if on {
        gfx.fill(rect.x, rect.y, rect.w, rect.h, 8.0, Color::rgb(0xF4F4F4));
        gfx.stroke(rect.x, rect.y, rect.w, rect.h, 8.0, 1.0, LINE);
    }
    let center = rect.center();
    text_center(gfx, text, center.x, center.y, 13.0, false, if on { TEXT } else { MUTED });
}

fn paint_clock(gfx: &mut Gfx, center: Pt, radius: f32) {
    let color = Color::rgb(0x8A8A8A);
    let width = (radius * 0.16).max(1.0);
    gfx.circle_stroke(center.x, center.y, radius, width, color);
    gfx.line(center.x, center.y, center.x, center.y - radius * 0.55, width, color);
    gfx.line(center.x, center.y, center.x + radius * 0.42, center.y + radius * 0.12, width, color);
}

fn paint_chevron(gfx: &mut Gfx, center: Pt) {
    let w = 6.5;
    let h = 3.6;
    gfx.path(
        &[
            (center.x - w, center.y - h * 0.2),
            (center.x, center.y + h),
            (center.x + w, center.y - h * 0.2),
        ],
        1.5,
        MUTED,
    );
}

fn popup_layout(_field: TimeField, anchor: Pt, canvas: Canvas) -> PopupLayout {
    let visible = visible_design(canvas);
    let mut x = anchor.x - 36.0;
    let mut y = anchor.y + 16.0;
    if x + POP_W > visible.right() - 8.0 {
        x = visible.right() - POP_W - 8.0;
    }
    if y + POP_H > visible.y + visible.h - 8.0 {
        y = (anchor.y - POP_H - 10.0).max(visible.y + 8.0);
    }
    x = x.max(visible.x + 8.0);
    y = y.max(visible.y + 8.0);
    let panel = Rect::new(x, y, POP_W, POP_H);
    let hours_top = panel.y + 34.0;
    let mut hours = [Rect::new(0.0, 0.0, 0.0, 0.0); 24];
    for hour in 0..24 {
        let col = hour % 6;
        let row = hour / 6;
        hours[hour] = Rect::new(
            panel.x + 12.0 + col as f32 * 44.0,
            hours_top + row as f32 * 34.0,
            36.0,
            26.0,
        );
    }
    let minutes_top = hours_top + 4.0 * 34.0 - 8.0 + 6.0;
    let mut minutes = [Rect::new(0.0, 0.0, 0.0, 0.0); 4];
    for index in 0..4 {
        minutes[index] = Rect::new(panel.x + 12.0 + index as f32 * 44.0, minutes_top, 36.0, 26.0);
    }
    PopupLayout { panel, hours, minutes }
}

fn visible_design(canvas: Canvas) -> Rect {
    Rect::new(
        -canvas.origin.0 / canvas.k,
        -canvas.origin.1 / canvas.k,
        canvas_client_w(canvas) / canvas.k,
        canvas_client_h(canvas) / canvas.k,
    )
}

fn canvas_client_w(canvas: Canvas) -> f32 {
    canvas.origin.0 * 2.0 + DESIGN_W * canvas.k
}

fn canvas_client_h(canvas: Canvas) -> f32 {
    canvas.origin.1 * 2.0 + DESIGN_H * canvas.k
}

fn clock_rect(field: Rect) -> Rect {
    let center = Pt::new(field.right() - 22.0, field.center().y);
    Rect::new(center.x - 14.0, center.y - 14.0, 28.0, 28.0)
}

fn chip_width(gfx: &mut Gfx, task: &Task) -> f32 {
    16.0 + 12.0 + gfx.measure(&format!("{}  {}", task.name, fmt_hhmm(task.start_min)), 14.5, false) + 14.0
}

fn arc_fill(index: usize, selected: bool) -> Color {
    let hex = SWATCH[index % SWATCH.len()];
    if selected {
        return Color::rgb(hex);
    }
    let mix = |channel: u32| ((channel * 212 + 255 * 43) / 255) as u8;
    Color {
        r: mix((hex >> 16) & 255) as f32 / 255.0,
        g: mix((hex >> 8) & 255) as f32 / 255.0,
        b: mix(hex & 255) as f32 / 255.0,
        a: 1.0,
    }
}

fn halo(index: usize) -> Color {
    let hex = SWATCH[index % SWATCH.len()];
    let mix = |channel: u32| (channel as f32 * 0.15 + 255.0 * 0.85).round() as u8;
    Color {
        r: mix((hex >> 16) & 255) as f32 / 255.0,
        g: mix((hex >> 8) & 255) as f32 / 255.0,
        b: mix(hex & 255) as f32 / 255.0,
        a: 1.0,
    }
}

fn text_left(gfx: &mut Gfx, text: &str, x: f32, y: f32, size: f32, bold: bool, color: Color) {
    gfx.text(text, x, y - size, 900.0, size * 2.0, size, bold, color, Align::Left, false);
}

fn text_center(gfx: &mut Gfx, text: &str, x: f32, y: f32, size: f32, bold: bool, color: Color) {
    gfx.text(text, x - 450.0, y - size, 900.0, size * 2.0, size, bold, color, Align::Center, false);
}

fn text_right(gfx: &mut Gfx, text: &str, x: f32, y: f32, size: f32, bold: bool, color: Color) {
    gfx.text(text, x - 450.0, y - size, 450.0, size * 2.0, size, bold, color, Align::Right, false);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn design_canvas_fits_and_centers() {
        let exact = canvas_for(DESIGN_W, DESIGN_H);
        assert!((exact.k - 1.0).abs() < 1e-4);
        assert!(exact.origin.0.abs() < 1e-3 && exact.origin.1.abs() < 1e-3);

        let fitted = canvas_for(980.0, 1040.0);
        let expect = (1040.0 / DESIGN_H).min(980.0 / DESIGN_W);
        assert!((fitted.k - expect).abs() < 1e-4);
        let used_h = DESIGN_H * fitted.k;
        assert!((fitted.origin.1 - (1040.0 - used_h) * 0.5).abs() < 1e-3);
        let back = to_design(fitted, fitted.origin.0, fitted.origin.1);
        assert!(back.x.abs() < 1e-3 && back.y.abs() < 1e-3);
    }
}
