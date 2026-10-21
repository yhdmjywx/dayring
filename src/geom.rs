//! Ring geometry in the reference screenshot's pixel space.
//! 00:00 is at the top and time increases clockwise.

use crate::model::{duration, Task, DAY, DAY_F, MIN_DURATION};

pub const DESIGN_W: f32 = 1020.0;
pub const DESIGN_H: f32 = 880.0;

/// Measured from the reference: arc centerline, stroke, and label orbit.
pub const RING_CENTER: Pt = Pt { x: 495.0, y: 422.0 };
pub const RING_RADIUS: f32 = 213.0;
pub const RING_STROKE: f32 = 44.0;
pub const HANDLE_R: f32 = 9.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pt {
    pub x: f32,
    pub y: f32,
}

impl Pt {
    pub fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    pub fn dist(self, other: Pt) -> f32 {
        (self.x - other.x).hypot(self.y - other.y)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    HandleStart(u64),
    HandleEnd(u64),
    Body(u64),
    /// On the ring band, but not on a task.
    Empty,
    Miss,
}

pub fn point_at(center: Pt, radius: f32, minutes: f32) -> Pt {
    let theta = (minutes / DAY_F) * std::f32::consts::TAU;
    Pt::new(
        center.x + radius * theta.sin(),
        center.y - radius * theta.cos(),
    )
}

pub fn minutes_at(center: Pt, point: Pt) -> f32 {
    let dx = point.x - center.x;
    let dy = point.y - center.y;
    let theta = dx.atan2(-dy);
    let turns = if theta < 0.0 {
        theta + std::f32::consts::TAU
    } else {
        theta
    };
    turns / std::f32::consts::TAU * DAY_F
}

/// Polyline stand-in for the ring. Drawing uses Direct2D arcs; tests still check this.
#[cfg_attr(not(test), allow(dead_code))]
pub fn arc_points(center: Pt, radius: f32, start_min: i32, end_min: i32) -> Vec<Pt> {
    let dur = duration(start_min, end_min).max(1) as f32;
    let steps = ((dur / DAY_F) * 144.0).clamp(8.0, 180.0) as u32;
    (0..=steps)
        .map(|i| {
            let minutes = start_min as f32 + dur * (i as f32 / steps as f32);
            point_at(center, radius, minutes)
        })
        .collect()
}

fn cap_slack_minutes() -> f32 {
    // Round caps extend half a stroke past each endpoint along the tangent.
    (RING_STROKE * 0.5 / RING_RADIUS) / std::f32::consts::TAU * DAY_F
}

fn angular_outside(minutes: f32, start: i32, end: i32) -> f32 {
    let rel = (minutes - start as f32).rem_euclid(DAY_F);
    let dur = duration(start, end) as f32;
    if rel <= dur {
        0.0
    } else {
        (rel - dur).min(DAY_F - rel)
    }
}

pub fn hit_test(tasks: &[Task], selected: Option<u64>, point: Pt) -> Hit {
    let center = RING_CENTER;
    if let Some(id) = selected {
        if let Some(task) = tasks.iter().find(|task| task.id == id) {
            let start = point_at(center, RING_RADIUS, task.start_min as f32);
            let end = point_at(center, RING_RADIUS, task.end_min as f32);
            let grab = HANDLE_R + 7.0;
            let ds = point.dist(start);
            let de = point.dist(end);
            if ds <= grab || de <= grab {
                return if ds <= de {
                    Hit::HandleStart(id)
                } else {
                    Hit::HandleEnd(id)
                };
            }
        }
    }

    let slack = cap_slack_minutes();
    let mut best: Option<(u64, f32)> = None;
    for task in tasks {
        let radial = (point.dist(center) - RING_RADIUS).abs();
        if radial > RING_STROKE * 0.5 + 8.0 {
            continue;
        }
        let outside = angular_outside(minutes_at(center, point), task.start_min, task.end_min);
        if outside > slack {
            continue;
        }
        let mut score = radial + outside * 0.2;
        if Some(task.id) == selected {
            score -= 3.0;
        }
        if best.map(|(_, best_score)| score < best_score).unwrap_or(true) {
            best = Some((task.id, score));
        }
    }
    if let Some((id, _)) = best {
        return Hit::Body(id);
    }

    let radial = (point.dist(center) - RING_RADIUS).abs();
    if radial <= RING_STROKE * 0.5 + 3.0 {
        Hit::Empty
    } else {
        Hit::Miss
    }
}

/// Keep a dragged endpoint from crossing the other one.
/// `previous` is the last accepted value, so a fast drag stays on the same side.
pub fn clamp_start(desired: i32, end: i32, previous: i32) -> i32 {
    clamp_endpoint(desired, end, previous, true)
}

pub fn clamp_end(desired: i32, start: i32, previous: i32) -> i32 {
    clamp_endpoint(desired, start, previous, false)
}

fn clamp_endpoint(desired: i32, fixed: i32, previous: i32, moving_start: bool) -> i32 {
    let desired = desired.rem_euclid(DAY);
    let dur = if moving_start {
        duration(desired, fixed)
    } else {
        duration(fixed, desired)
    };
    if (MIN_DURATION..=DAY - MIN_DURATION).contains(&dur) {
        return desired;
    }
    let prev_dur = if moving_start {
        duration(previous, fixed)
    } else {
        duration(fixed, previous)
    };
    if prev_dur <= DAY / 2 {
        if moving_start {
            (fixed - MIN_DURATION).rem_euclid(DAY)
        } else {
            (fixed + MIN_DURATION).rem_euclid(DAY)
        }
    } else if moving_start {
        (fixed + MIN_DURATION).rem_euclid(DAY)
    } else {
        (fixed - MIN_DURATION).rem_euclid(DAY)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Schedule;

    fn task(id: u64, start: i32, end: i32) -> Task {
        Task {
            id,
            name: format!("t{id}"),
            start_min: start,
            end_min: end,
            color: 0,
        }
    }

    #[test]
    fn arc_points_meet_the_endpoints() {
        let points = arc_points(RING_CENTER, RING_RADIUS, 9 * 60, 11 * 60 + 30);
        let end = point_at(RING_CENTER, RING_RADIUS, (11 * 60 + 30) as f32);
        assert!(points.len() >= 8);
        assert!(points[0].dist(point_at(RING_CENTER, RING_RADIUS, (9 * 60) as f32)) < 0.01);
        assert!(points.last().unwrap().dist(end) < 0.01);
    }

    #[test]
    fn minutes_roundtrip_on_the_quarter_hour() {
        for minutes in (0..DAY).step_by(15) {
            let point = point_at(RING_CENTER, RING_RADIUS, minutes as f32);
            let back = minutes_at(RING_CENTER, point);
            let delta = (back - minutes as f32).rem_euclid(DAY_F).min(
                (minutes as f32 - back).rem_euclid(DAY_F),
            );
            assert!(delta < 0.05, "{minutes} came back as {back}");
        }
    }

    #[test]
    fn midnight_is_the_top_and_six_is_the_right() {
        let top = point_at(RING_CENTER, 100.0, 0.0);
        assert!(top.y < RING_CENTER.y);
        assert!((top.x - RING_CENTER.x).abs() < 0.01);
        let right = point_at(RING_CENTER, 100.0, 6.0 * 60.0);
        assert!(right.x > RING_CENTER.x);
        assert!((right.y - RING_CENTER.y).abs() < 0.01);
    }

    #[test]
    fn hit_demo_arcs_and_the_empty_band() {
        let schedule = Schedule::demo();
        let tasks = schedule.tasks();
        let selected = schedule.selected_id();
        let create = point_at(RING_CENTER, RING_RADIUS, 10.0 * 60.0 + 15.0);
        assert_eq!(hit_test(tasks, selected, create), Hit::Body(2));
        let empty_at_1830 = point_at(RING_CENTER, RING_RADIUS, (18 * 60 + 30) as f32);
        assert_eq!(hit_test(tasks, selected, empty_at_1830), Hit::Empty);
        let center = RING_CENTER;
        assert_eq!(hit_test(tasks, selected, center), Hit::Miss);
    }

    #[test]
    fn hit_selected_handle_before_the_body() {
        let schedule = Schedule::demo();
        let start = point_at(RING_CENTER, RING_RADIUS, (9 * 60) as f32);
        assert!(matches!(
            hit_test(schedule.tasks(), Some(2), start),
            Hit::HandleStart(2)
        ));
    }

    #[test]
    fn hit_task_that_crosses_midnight() {
        let tasks = vec![task(7, 23 * 60, 60)];
        let at_midnight = point_at(RING_CENTER, RING_RADIUS, 0.0);
        assert_eq!(hit_test(&tasks, None, at_midnight), Hit::Body(7));
        let at_noon = point_at(RING_CENTER, RING_RADIUS, (12 * 60) as f32);
        assert_eq!(hit_test(&tasks, None, at_noon), Hit::Empty);
    }

    #[test]
    fn resize_cannot_cross_the_other_end() {
        let end = 10 * 60;
        let shrunk = clamp_start(end, end, 9 * 60);
        assert_eq!(duration(shrunk, end), MIN_DURATION);
        let grown = clamp_end(23 * 60 + 50, 23 * 60, 23 * 60 + 30);
        assert!(duration(23 * 60, grown) >= MIN_DURATION);
    }

    #[test]
    fn resize_can_cross_midnight_without_collapsing() {
        let start = 23 * 60;
        let end = clamp_end(60, start, 23 * 60 + 30);
        assert_eq!(end, 60);
        assert_eq!(duration(start, end), 2 * 60);
    }
}
