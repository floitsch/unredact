// Copyright (C) 2026 Toit contributors.
//! Conservative OCR line continuation, independent of candidate identities.
use crate::{
    contextual::{Context, visible_context},
    project::{Project, Target},
    raster::Rect,
};
use anyhow::{Result, ensure};
use std::collections::BTreeMap;

#[derive(Clone)]
struct Line {
    id: String,
    words: Vec<usize>,
    bounds: Rect,
    height: f64,
}
fn union(a: Rect, b: Rect) -> Rect {
    let x = a.x.min(b.x);
    let y = a.y.min(b.y);
    Rect {
        x,
        y,
        w: (a.x + a.w).max(b.x + b.w) - x,
        h: (a.y + a.h).max(b.y + b.h) - y,
    }
}
fn row(a: Rect, b: Rect) -> bool {
    (a.y + a.h).min(b.y + b.h) - a.y.max(b.y) >= a.h.min(b.h) * 0.5
}

/// Up to twelve words and two line transitions per side. Supplied context is
/// left intact; every OCR word contributing to an extension is recorded.
pub fn paragraph_context(project: &Project, target: &Target) -> Result<Context> {
    let mut result = visible_context(project, target)?;
    if result.left_source == "supplied" && result.right_source == "supplied" {
        return Ok(result);
    }
    let page = project.page(&target.page)?;
    let valid =
        |b: Rect| b.valid() && b.x + b.w <= page.width as f64 && b.y + b.h <= page.height as f64;
    let near: Vec<_> = page
        .words
        .iter()
        .enumerate()
        .filter(|(_, w)| valid(w.bounds) && row(w.bounds, target.bounds))
        .collect();
    let left = near
        .iter()
        .filter(|(_, w)| w.bounds.x + w.bounds.w <= target.bounds.x)
        .max_by(|a, b| (a.1.bounds.x + a.1.bounds.w).total_cmp(&(b.1.bounds.x + b.1.bounds.w)))
        .filter(|(_, w)| target.bounds.x - w.bounds.x - w.bounds.w <= target.bounds.h * 4.0);
    let right = near
        .iter()
        .filter(|(_, w)| w.bounds.x >= target.bounds.x + target.bounds.w)
        .min_by(|a, b| a.1.bounds.x.total_cmp(&b.1.bounds.x))
        .filter(|(_, w)| w.bounds.x - target.bounds.x - target.bounds.w <= target.bounds.h * 4.0);
    let seeds: Vec<_> = [left, right]
        .into_iter()
        .flatten()
        .map(|(_, w)| (*w).clone())
        .collect();
    if seeds.is_empty()
        || seeds.iter().any(|w| {
            w.block.is_empty()
                || w.line.is_empty()
                || w.block != seeds[0].block
                || w.line != seeds[0].line
        })
    {
        result
            .layout_notes
            .push("No unambiguous nearby OCR paragraph/line; kept same-row context".into());
        return Ok(result);
    }
    let mut grouped: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (i, w) in page
        .words
        .iter()
        .enumerate()
        .filter(|(_, w)| w.block == seeds[0].block && valid(w.bounds))
    {
        grouped.entry(w.line.clone()).or_default().push(i);
    }
    let mut lines = vec![];
    for (id, mut words) in grouped {
        words.sort_by(|a, b| page.words[*a].bounds.x.total_cmp(&page.words[*b].bounds.x));
        let mut bounds = page.words[words[0]].bounds;
        let mut heights = vec![];
        for &i in &words {
            bounds = union(bounds, page.words[i].bounds);
            heights.push(page.words[i].bounds.h);
        }
        if id == seeds[0].line {
            bounds = union(bounds, target.bounds);
        }
        heights.sort_by(f64::total_cmp);
        lines.push(Line {
            id,
            words,
            bounds,
            height: heights[heights.len() / 2],
        });
    }
    lines.sort_by(|a, b| a.bounds.y.total_cmp(&b.bounds.y));
    let seed = lines.iter().position(|l| l.id == seeds[0].line).unwrap();
    if lines[seed].bounds.h > lines[seed].height * 2.0 {
        result
            .layout_notes
            .push("OCR line spans incompatible vertical positions; kept same-row context".into());
        return Ok(result);
    }
    let column_left = lines
        .iter()
        .map(|l| l.bounds.x)
        .fold(f64::INFINITY, f64::min);
    let column_right = lines
        .iter()
        .map(|l| l.bounds.x + l.bounds.w)
        .fold(0.0, f64::max);
    let masks: Vec<_> = page
        .proposed_redactions
        .iter()
        .copied()
        .chain(
            project
                .targets
                .iter()
                .filter(|t| t.page == target.page)
                .map(|t| t.bounds),
        )
        .collect();
    let masked = |x0: f64, x1: f64, bounds: Rect| {
        x1 > x0
            && masks.iter().any(|m| {
                m.intersects(Rect {
                    x: x0,
                    y: bounds.y,
                    w: x1 - x0,
                    h: bounds.h,
                })
            })
    };
    // A merged multi-column OCR line can have a large unaccounted gutter.
    let gutter = |line: &Line| {
        line.words.windows(2).any(|pair| {
            let a = page.words[pair[0]].bounds;
            let b = page.words[pair[1]].bounds;
            b.x - a.x - a.w > line.height * 4.0 && !masked(a.x + a.w, b.x, line.bounds)
        })
    };
    let walk = |left: bool| {
        let mut line_index = seed;
        let mut boundary = if left {
            target.bounds.x
        } else {
            target.bounds.x + target.bounds.w
        };
        let mut selected = vec![];
        let mut notes = vec![];
        let side = if left { "left" } else { "right" };
        let mut transitions = 0;
        'walk: loop {
            let line = &lines[line_index];
            let mut words = line.words.clone();
            let start_boundary = boundary;
            if left {
                words.reverse();
            }
            for i in words {
                let w = &page.words[i];
                let b = w.bounds;
                if (left && b.x + b.w > start_boundary) || (!left && b.x < start_boundary) {
                    continue;
                }
                let (x0, x1, gap) = if left {
                    (b.x, boundary, boundary - b.x - b.w)
                } else {
                    (boundary, b.x + b.w, b.x - boundary)
                };
                if !w.confidence.is_finite()
                    || w.confidence < 50.0
                    || w.text.is_empty()
                    || w.text.chars().any(char::is_control)
                    || gap < 0.0
                    || gap > line.height * 4.0
                    || masks.iter().any(|m| m.intersects(b))
                    || masked(x0, x1, line.bounds)
                {
                    notes.push(format!(
                        "{side}: stopped at masked, unreliable or separated text on {}",
                        line.id
                    ));
                    break 'walk;
                }
                selected.push(i);
                boundary = if left { b.x } else { b.x + b.w };
                if selected.len() == 12 {
                    break 'walk;
                }
            }
            if transitions == 2 {
                notes.push(format!("{side}: two-line continuation limit"));
                break;
            }
            let next = if left {
                line_index.checked_sub(1)
            } else {
                (line_index + 1 < lines.len()).then_some(line_index + 1)
            };
            let Some(next) = next else { break };
            let adjacent = &lines[next];
            let (earlier, later) = if left {
                (adjacent, line)
            } else {
                (line, adjacent)
            };
            let scale = line.height.max(adjacent.height);
            let pitch = later.bounds.y - earlier.bounds.y;
            let same_scale = line.height.min(adjacent.height) >= scale * 0.6;
            let full_earlier = column_right - earlier.bounds.x - earlier.bounds.w <= scale * 3.0;
            let aligned_later = later.bounds.x - column_left <= scale * 2.0;
            // Same paragraph identity alone does not establish line order.
            if !same_scale
                || pitch < scale * 0.5
                || pitch > scale * 2.5
                || !full_earlier
                || !aligned_later
                || gutter(line)
                || gutter(adjacent)
                || line.bounds.h > line.height * 2.0
                || adjacent.bounds.h > adjacent.height * 2.0
            {
                notes.push(format!(
                    "{side}: rejected line continuation {} -> {} by spacing/column checks",
                    line.id, adjacent.id
                ));
                break;
            }
            let (x0, x1) = if left {
                ((column_left - scale * 2.0).max(0.0), boundary)
            } else {
                (
                    boundary,
                    (column_right + scale * 2.0).min(page.width as f64),
                )
            };
            let (d0, d1) = if left {
                (
                    adjacent.bounds.x + adjacent.bounds.w,
                    (column_right + scale * 2.0).min(page.width as f64),
                )
            } else {
                ((column_left - scale * 2.0).max(0.0), adjacent.bounds.x)
            };
            if masked(x0, x1, line.bounds) || masked(d0, d1, adjacent.bounds) {
                notes.push(format!("{side}: mask at line edge before {}", adjacent.id));
                break;
            }
            // Joining a hyphenated word changes transcription; leave it to a
            // reviewed context rather than guessing whether the hyphen is lexical.
            let tail = &page.words[*earlier.words.last().unwrap()].text;
            if tail.ends_with('-') || tail.ends_with('\u{00ad}') {
                notes.push(format!("{side}: unresolved line-end hyphen"));
                break;
            }
            notes.push(format!(
                "{side}: continued {} -> {} within paragraph {}",
                line.id, adjacent.id, seeds[0].block
            ));
            line_index = next;
            boundary = if left { column_right } else { column_left };
            transitions += 1;
        }
        if left {
            selected.reverse();
        }
        (selected, notes)
    };
    for left in [true, false] {
        if (left && result.left_source == "supplied")
            || (!left && result.right_source == "supplied")
        {
            continue;
        }
        let (indices, notes) = walk(left);
        let text = indices
            .iter()
            .map(|&i| page.words[i].text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        if left {
            result.left = text;
            result.left_word_indices = indices;
            result.left_source = "ocr_paragraph".into();
        } else {
            result.right = text;
            result.right_word_indices = indices;
            result.right_source = "ocr_paragraph".into();
        }
        result.layout_notes.extend(notes);
    }
    ensure!(
        result.left.len() <= 4096 && result.right.len() <= 4096,
        "Context exceeds 4096 bytes per side"
    );
    Ok(result)
}
