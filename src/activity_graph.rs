use crate::db::activity::MembershipDay;
use plotters::prelude::*;
use std::collections::HashSet;

pub const DAY: i64 = 86400;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Coverage {
    Unavailable,
    Partial,
    Complete,
}

#[derive(Debug, Clone)]
pub struct ActivityDay {
    pub timestamp: i64,
    pub messages: u64,
    pub active_members: u64,
    pub member_count: Option<u64>,
    pub joins: Option<u64>,
    pub departures: Option<u64>,
    pub coverage: Coverage,
}

#[derive(Debug, Clone)]
pub struct ActivityGraphSnapshot {
    pub channel_name: String,
    pub captured_at: i64,
    pub days: Vec<ActivityDay>,
}

pub fn window_bounds(now: i64, days: u32) -> (i64, i64) {
    let today = now.div_euclid(DAY) * DAY;
    (today - i64::from(days.saturating_sub(1)) * DAY, today + DAY)
}

pub fn message_day(
    timestamp: i64,
    start: i64,
    end: i64,
    bot: bool,
    webhook: bool,
) -> Option<usize> {
    (!bot && !webhook && timestamp >= start && timestamp < end)
        .then(|| ((timestamp - start) / DAY) as usize)
}

pub fn build_snapshot(
    channel_name: String,
    now: i64,
    start: i64,
    messages: Vec<u64>,
    authors: Vec<HashSet<u64>>,
    membership: &[MembershipDay],
    intervals: &[(i64, i64)],
) -> ActivityGraphSnapshot {
    let days = messages
        .into_iter()
        .enumerate()
        .map(|(i, messages)| {
            let timestamp = start + i as i64 * DAY;
            let end = (timestamp + DAY).min(now);
            // Merge intervals to prevent overlap from inflating coverage.
            let mut cursor = timestamp;
            let mut covered = 0;
            let mut sorted = intervals.to_vec();
            sorted.sort_unstable();
            for (a, b) in sorted {
                let a = a.max(cursor).max(timestamp);
                let b = b.min(end);
                if b > a {
                    covered += b - a;
                    cursor = b;
                }
            }
            let row = membership.iter().find(|d| d.day == timestamp);
            let observed =
                row.is_some_and(|r| r.observed_at.is_some() || r.joins > 0 || r.departures > 0);
            let coverage = if covered == end - timestamp && covered > 0 {
                Coverage::Complete
            } else if covered > 0 || observed {
                Coverage::Partial
            } else {
                Coverage::Unavailable
            };
            ActivityDay {
                timestamp,
                messages,
                active_members: authors[i].len() as u64,
                member_count: row.and_then(|r| r.member_count).map(|n| n as u64),
                joins: (coverage != Coverage::Unavailable)
                    .then_some(row.map_or(0, |r| r.joins as u64)),
                departures: (coverage != Coverage::Unavailable)
                    .then_some(row.map_or(0, |r| r.departures as u64)),
                coverage,
            }
        })
        .collect();
    ActivityGraphSnapshot {
        channel_name,
        captured_at: now,
        days,
    }
}

pub const TITLES: [&str; 3] = [
    "Messages vs active members",
    "Messages vs total membership",
    "Messages vs joins and departures",
];

pub fn render(snapshot: &ActivityGraphSnapshot, metric: usize) -> anyhow::Result<Vec<u8>> {
    let (width, height) = (1200u32, 600u32);
    let mut pixels = vec![255; (width * height * 3) as usize];
    {
        let root = BitMapBackend::with_buffer(&mut pixels, (width, height)).into_drawing_area();
        root.fill(&WHITE)?;
        let n = snapshot.days.len();
        let primary: Vec<Option<u64>> = snapshot
            .days
            .iter()
            .map(|d| match metric {
                0 => Some(d.active_members),
                1 => d.member_count,
                _ => d.joins,
            })
            .collect();
        let secondary: Vec<Option<u64>> = snapshot.days.iter().map(|d| d.departures).collect();
        let max_messages = snapshot
            .days
            .iter()
            .map(|d| d.messages)
            .max()
            .unwrap_or(0)
            .max(1) as f64
            * 1.15;
        let max_members = primary
            .iter()
            .chain(if metric == 2 {
                secondary.iter()
            } else {
                primary.iter()
            })
            .flatten()
            .copied()
            .max()
            .unwrap_or(0)
            .max(1) as f64
            * 1.15;
        let member_label = match metric {
            0 => "Active members",
            1 => "Last observed members",
            _ => "Joins / departures",
        };
        let mut chart = ChartBuilder::on(&root)
            .caption(
                format!("#{}: {}", snapshot.channel_name, TITLES[metric]),
                ("sans-serif", 26),
            )
            .margin(20)
            .x_label_area_size(80)
            .y_label_area_size(65)
            .right_y_label_area_size(65)
            .build_cartesian_2d(0f64..n as f64, 0f64..max_messages)?
            .set_secondary_coord(0f64..n as f64, 0f64..max_members);
        chart
            .configure_mesh()
            .x_desc("UTC date (* today; ~ partial; ? unavailable)")
            .y_desc("Human messages")
            .x_labels(n.min(10))
            .x_label_formatter(&|x| {
                let index = x.floor() as usize;
                snapshot
                    .days
                    .get(index)
                    .map(|d| {
                        let date = time::OffsetDateTime::from_unix_timestamp(d.timestamp)
                            .unwrap()
                            .date();
                        let today =
                            snapshot.captured_at.div_euclid(DAY) == d.timestamp.div_euclid(DAY);
                        let marker = if metric == 0 {
                            ""
                        } else {
                            match d.coverage {
                                Coverage::Partial => "~",
                                Coverage::Unavailable => "?",
                                Coverage::Complete => "",
                            }
                        };
                        format!(
                            "{:02}-{:02}{}{}",
                            u8::from(date.month()),
                            date.day(),
                            if today { "*" } else { "" },
                            marker
                        )
                    })
                    .unwrap_or_default()
            })
            .draw()?;
        chart
            .configure_secondary_axes()
            .y_desc(member_label)
            .draw()?;
        if metric != 0 {
            for (coverage, color, label) in [
                (
                    Coverage::Partial,
                    RGBColor(255, 243, 205),
                    "Partial coverage",
                ),
                (
                    Coverage::Unavailable,
                    RGBColor(235, 235, 235),
                    "Unavailable",
                ),
            ] {
                if snapshot.days.iter().any(|day| day.coverage == coverage) {
                    chart
                        .draw_series(
                            snapshot
                                .days
                                .iter()
                                .enumerate()
                                .filter(|(_, day)| day.coverage == coverage)
                                .map(|(i, _)| {
                                    Rectangle::new(
                                        [(i as f64, 0.0), (i as f64 + 1.0, max_messages)],
                                        color.filled(),
                                    )
                                }),
                        )?
                        .label(label)
                        .legend(move |(x, y)| {
                            Rectangle::new([(x, y - 5), (x + 20, y + 5)], color.filled())
                        });
                }
            }
        }
        chart
            .draw_series(snapshot.days.iter().enumerate().map(|(i, day)| {
                Rectangle::new(
                    [
                        (i as f64 + 0.15, 0.0),
                        (i as f64 + 0.85, day.messages as f64),
                    ],
                    BLUE.mix(0.35).filled(),
                )
            }))?
            .label("Human messages")
            .legend(|(x, y)| {
                Rectangle::new([(x, y - 5), (x + 20, y + 5)], BLUE.mix(0.35).filled())
            });
        // Separate contiguous segments: never connect a line through missing observations.
        for (values, color, label) in [
            (&primary, GREEN, member_label),
            (&secondary, RED, "Departures"),
        ] {
            if metric != 2 && label == "Departures" {
                continue;
            }
            let mut segment = Vec::new();
            for i in 0..=n {
                if i < n
                    && let Some(value) = values[i]
                {
                    segment.push((i as f64 + 0.5, value as f64));
                    continue;
                }
                if !segment.is_empty() {
                    chart.draw_secondary_series(LineSeries::new(
                        segment.clone(),
                        color.stroke_width(3),
                    ))?;
                    segment.clear();
                }
            }
            chart
                .draw_secondary_series(values.iter().enumerate().filter_map(|(i, value)| {
                    value.map(|v| Circle::new((i as f64 + 0.5, v as f64), 4, color.filled()))
                }))?
                .label(if metric == 2 && color == GREEN {
                    "Joins"
                } else {
                    label
                })
                .legend(move |(x, y)| {
                    PathElement::new([(x, y), (x + 20, y)], color.stroke_width(3))
                });
        }
        chart
            .configure_series_labels()
            .background_style(WHITE.mix(0.9))
            .border_style(BLACK)
            .draw()?;
        root.present()?;
    }
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, width, height);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header()?.write_image_data(&pixels)?;
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn utc_window_and_human_message_boundaries() {
        assert_eq!(window_bounds(DAY * 40 + 10, 7), (DAY * 34, DAY * 41));
        assert_eq!(message_day(100, 100, 200, false, false), Some(0));
        assert_eq!(message_day(200, 100, 200, false, false), None);
        assert_eq!(message_day(99, 100, 200, false, false), None);
        assert_eq!(message_day(100, 100, 200, true, false), None);
        assert_eq!(message_day(100, 100, 200, false, true), None);
    }
    #[test]
    fn coverage_and_distinct_authors() {
        let authors = vec![HashSet::from([1, 1, 2]), HashSet::new(), HashSet::new()];
        let snap = build_snapshot(
            "general".into(),
            DAY * 3,
            0,
            vec![3, 0, 0],
            authors,
            &[],
            &[(0, DAY), (DAY / 2, DAY), (DAY * 2, DAY * 2 + 60)],
        );
        assert_eq!(snap.days[0].active_members, 2);
        assert_eq!(snap.days[0].coverage, Coverage::Complete);
        assert_eq!(snap.days[1].coverage, Coverage::Unavailable);
        assert_eq!(snap.days[1].joins, None);
        assert_eq!(snap.days[2].coverage, Coverage::Partial);
    }
    #[test]
    fn render_empty_and_scaled_fixtures() -> anyhow::Result<()> {
        let mut snap = build_snapshot(
            "general".into(),
            DAY * 7 + 3600,
            DAY,
            vec![0; 7],
            vec![HashSet::new(); 7],
            &[],
            &[],
        );
        for fixture in 0..2 {
            if fixture == 1 {
                snap.days[0].messages = 10000;
                for day in &mut snap.days {
                    day.member_count = Some(50);
                    day.joins = Some(0);
                    day.departures = Some(1);
                }
                snap.days[2].member_count = None;
            }
            for metric in 0..3 {
                let bytes = render(&snap, metric)?;
                let decoder = png::Decoder::new(std::io::Cursor::new(&bytes));
                let mut reader = decoder.read_info()?;
                let mut output = vec![0; reader.output_buffer_size()];
                let info = reader.next_frame(&mut output)?;
                assert_eq!((info.width, info.height), (1200, 600));
                if let Ok(dir) = std::env::var("MODBOT_GRAPH_PREVIEW_DIR") {
                    std::fs::write(
                        std::path::Path::new(&dir).join(format!("fixture-{fixture}-{metric}.png")),
                        bytes,
                    )?;
                }
            }
        }
        Ok(())
    }
}
