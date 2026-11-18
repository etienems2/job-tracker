use std::collections::{BTreeMap, HashMap};

use crate::domain::{Application, Stage};

const COLUMN_WIDTH: u16 = 16;
const GUTTER: u16 = 10;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FlowNode {
    pub(crate) step: usize,
    pub(crate) stage: Stage,
    pub(crate) count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FlowLink {
    pub(crate) from_step: usize,
    pub(crate) from_stage: Stage,
    pub(crate) to_stage: Stage,
    pub(crate) count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Flow {
    pub(crate) nodes: Vec<FlowNode>,
    pub(crate) links: Vec<FlowLink>,
    pub(crate) applications: usize,
    pub(crate) steps: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PlacedNode {
    pub(crate) step: usize,
    pub(crate) stage: Stage,
    pub(crate) count: usize,
    pub(crate) x: u16,
    pub(crate) y: u16,
    pub(crate) width: u16,
    pub(crate) height: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PlacedRibbon {
    pub(crate) stage: Stage,
    pub(crate) x: u16,
    pub(crate) width: u16,
    pub(crate) src_y: u16,
    pub(crate) src_height: u16,
    pub(crate) dst_y: u16,
    pub(crate) dst_height: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Chart {
    pub(crate) title: String,
    pub(crate) nodes: Vec<PlacedNode>,
    pub(crate) ribbons: Vec<PlacedRibbon>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Placement {
    Empty,
    TooSmall,
    Chart(Chart),
}

pub(crate) fn flow(apps: &[Application]) -> Flow {
    let mut node_counts: BTreeMap<(usize, u8), usize> = BTreeMap::new();
    let mut link_counts: BTreeMap<(usize, u8, u8), usize> = BTreeMap::new();
    let mut steps = 0;
    for app in apps {
        steps = steps.max(app.events.len());
        for (step, event) in app.events.iter().enumerate() {
            *node_counts
                .entry((step, event.stage.pipeline_rank()))
                .or_default() += 1;
        }
        for (step, pair) in app.events.windows(2).enumerate() {
            *link_counts
                .entry((
                    step,
                    pair[0].stage.pipeline_rank(),
                    pair[1].stage.pipeline_rank(),
                ))
                .or_default() += 1;
        }
    }

    Flow {
        nodes: node_counts
            .into_iter()
            .map(|((step, rank), count)| FlowNode {
                step,
                stage: stage_by_rank(rank),
                count,
            })
            .collect(),
        links: link_counts
            .into_iter()
            .map(|((from_step, from_rank, to_rank), count)| FlowLink {
                from_step,
                from_stage: stage_by_rank(from_rank),
                to_stage: stage_by_rank(to_rank),
                count,
            })
            .collect(),
        applications: apps.len(),
        steps,
    }
}

pub(crate) fn layout(flow: &Flow, width: u16, height: u16) -> Placement {
    if flow.steps == 0 || flow.nodes.is_empty() {
        return Placement::Empty;
    }
    let Some(content_height) = height.checked_sub(3).filter(|inner| *inner > 0) else {
        return Placement::TooSmall;
    };
    let inner_width = width.saturating_sub(2);
    let shown = columns_that_fit(inner_width, flow.steps);
    if shown == 0 {
        return Placement::TooSmall;
    }

    let mut column_nodes: Vec<Vec<&FlowNode>> = Vec::with_capacity(shown);
    for step in 0..shown {
        let column: Vec<&FlowNode> = flow.nodes.iter().filter(|node| node.step == step).collect();
        let gaps = column.len().saturating_sub(1) as u16;
        let needed = column.len() as u16 + gaps;
        if content_height < needed {
            return Placement::TooSmall;
        }
        column_nodes.push(column);
    }
    let max_sum = column_nodes
        .iter()
        .map(|column| column.iter().map(|node| node.count).sum::<usize>())
        .max()
        .unwrap_or(1)
        .max(1);
    let fullest_gaps = column_nodes
        .iter()
        .filter(|column| column.iter().map(|node| node.count).sum::<usize>() == max_sum)
        .map(|column| column.len().saturating_sub(1))
        .max()
        .unwrap_or(0);
    let node_rows = usize::from(content_height) - fullest_gaps;
    let gutter = gutter_width(inner_width, shown);

    let mut columns: Vec<Vec<PlacedNode>> = Vec::with_capacity(shown);
    let mut x = 0u16;
    for (step, nodes) in column_nodes.iter().enumerate() {
        columns.push(place_column(nodes, x, max_sum, node_rows));
        x = x.saturating_add(COLUMN_WIDTH);
        if step + 1 < shown {
            x = x.saturating_add(gutter);
        }
    }

    let ribbons = place_ribbons(flow, &columns, shown, gutter);
    let nodes = columns.into_iter().flatten().collect();
    Placement::Chart(Chart {
        title: chart_title(flow.applications, shown, flow.steps),
        nodes,
        ribbons,
    })
}

fn chart_title(applications: usize, shown: usize, steps: usize) -> String {
    if shown < steps {
        format!("Flow · {applications} · showing first {shown} of {steps} steps")
    } else {
        format!("Flow · {applications}")
    }
}

fn columns_that_fit(inner_width: u16, steps: usize) -> usize {
    let mut shown = 0usize;
    let mut used = 0u16;
    for _ in 0..steps {
        let gap = if shown == 0 { 0 } else { GUTTER };
        let next = used.saturating_add(gap).saturating_add(COLUMN_WIDTH);
        if next > inner_width {
            break;
        }
        used = next;
        shown += 1;
    }
    shown
}

fn place_column(nodes: &[&FlowNode], x: u16, max_sum: usize, node_rows: usize) -> Vec<PlacedNode> {
    let weights: Vec<usize> = nodes.iter().map(|node| node.count).collect();
    let heights = shared_heights(&weights, max_sum, node_rows);
    let mut y = 0u16;
    let mut placed = Vec::with_capacity(nodes.len());
    for (node, height) in nodes.iter().zip(heights) {
        let height = height as u16;
        placed.push(PlacedNode {
            step: node.step,
            stage: node.stage,
            count: node.count,
            x,
            y,
            width: COLUMN_WIDTH,
            height,
        });
        y = y.saturating_add(height).saturating_add(1);
    }
    placed
}

fn gutter_width(inner_width: u16, shown: usize) -> u16 {
    let gutters = shown.saturating_sub(1);
    if gutters == 0 {
        return 0;
    }
    let used = COLUMN_WIDTH
        .saturating_mul(shown as u16)
        .saturating_add(GUTTER.saturating_mul(gutters as u16));
    let extra = inner_width.saturating_sub(used);
    GUTTER + extra / gutters as u16
}

fn shared_heights(weights: &[usize], max_sum: usize, node_rows: usize) -> Vec<usize> {
    if weights.is_empty() {
        return Vec::new();
    }
    let mut heights: Vec<usize> = weights
        .iter()
        .map(|count| (count.saturating_mul(node_rows) / max_sum).max(1))
        .collect();
    let mut sum: usize = heights.iter().sum();
    while sum > node_rows {
        let Some(index) = heights
            .iter()
            .enumerate()
            .filter(|(_, height)| **height > 1)
            .max_by_key(|(index, height)| (*height, std::cmp::Reverse(*index)))
            .map(|(index, _)| index)
        else {
            break;
        };
        heights[index] -= 1;
        sum -= 1;
    }
    heights
}

fn place_ribbons(
    flow: &Flow,
    columns: &[Vec<PlacedNode>],
    shown: usize,
    gutter: u16,
) -> Vec<PlacedRibbon> {
    let mut source_slices: HashMap<(usize, u8, u8), (u16, u16)> = HashMap::new();
    let mut target_slices: HashMap<(usize, u8, u8), (u16, u16)> = HashMap::new();

    for step in 0..shown.saturating_sub(1) {
        for source in &columns[step] {
            let mut outgoing: Vec<&FlowLink> = flow
                .links
                .iter()
                .filter(|link| {
                    link.from_step == step && link.from_stage == source.stage && step + 1 < shown
                })
                .collect();
            outgoing.sort_by_key(|link| link.to_stage.pipeline_rank());
            let leaving: usize = outgoing.iter().map(|link| link.count).sum();
            let stay = source.count.saturating_sub(leaving);
            let mut weights: Vec<usize> = outgoing.iter().map(|link| link.count).collect();
            if stay > 0 {
                weights.push(stay);
            }
            let ranges = stacked_ranges(source.y, source.height, &weights);
            for (link, range) in outgoing.iter().zip(ranges) {
                source_slices.insert(
                    (
                        step,
                        link.from_stage.pipeline_rank(),
                        link.to_stage.pipeline_rank(),
                    ),
                    range,
                );
            }
        }

        let target_step = step + 1;
        for target in &columns[target_step] {
            let mut incoming: Vec<&FlowLink> = flow
                .links
                .iter()
                .filter(|link| link.from_step == step && link.to_stage == target.stage)
                .collect();
            incoming.sort_by_key(|link| link.from_stage.pipeline_rank());
            let weights: Vec<usize> = incoming.iter().map(|link| link.count).collect();
            let ranges = stacked_ranges(target.y, target.height, &weights);
            for (link, range) in incoming.iter().zip(ranges) {
                target_slices.insert(
                    (
                        step,
                        link.from_stage.pipeline_rank(),
                        link.to_stage.pipeline_rank(),
                    ),
                    range,
                );
            }
        }
    }

    let mut ribbons = Vec::new();
    for link in &flow.links {
        if link.from_step + 1 >= shown {
            continue;
        }
        let key = (
            link.from_step,
            link.from_stage.pipeline_rank(),
            link.to_stage.pipeline_rank(),
        );
        let Some((src_y, src_height)) = source_slices.get(&key).copied() else {
            continue;
        };
        let Some((dst_y, dst_height)) = target_slices.get(&key).copied() else {
            continue;
        };
        ribbons.push(PlacedRibbon {
            stage: link.from_stage,
            x: gutter_x(link.from_step, gutter),
            width: gutter,
            src_y,
            src_height,
            dst_y,
            dst_height,
        });
    }
    ribbons
}

fn gutter_x(step: usize, gutter: u16) -> u16 {
    let step = u16::try_from(step).unwrap_or(u16::MAX);
    step.saturating_mul(COLUMN_WIDTH.saturating_add(gutter))
        .saturating_add(COLUMN_WIDTH)
}

fn distribute(weights: &[usize], rows: usize) -> Vec<usize> {
    let count = weights.len();
    if count == 0 || rows == 0 {
        return vec![0; count];
    }
    if rows < count {
        let mut heights = vec![0; count];
        let mut order: Vec<usize> = (0..count).collect();
        order.sort_by(|&left, &right| weights[right].cmp(&weights[left]).then(left.cmp(&right)));
        for index in order.into_iter().take(rows) {
            heights[index] = 1;
        }
        return heights;
    }

    let mut heights = vec![1; count];
    let extra = rows - count;
    let total: usize = weights.iter().sum();
    if extra == 0 || total == 0 {
        return heights;
    }

    let mut given = 0usize;
    let mut extra_rows = vec![0usize; count];
    for (index, weight) in weights.iter().enumerate() {
        let add = weight.saturating_mul(extra) / total;
        extra_rows[index] = add;
        given += add;
    }
    let mut left = extra - given;
    let mut order: Vec<usize> = (0..count).collect();
    order.sort_by(|&left_index, &right_index| {
        weights[right_index]
            .cmp(&weights[left_index])
            .then(left_index.cmp(&right_index))
    });
    for index in order {
        if left == 0 {
            break;
        }
        extra_rows[index] += 1;
        left -= 1;
    }
    for index in 0..count {
        heights[index] += extra_rows[index];
    }
    heights
}

fn stacked_ranges(origin: u16, height: u16, weights: &[usize]) -> Vec<(u16, u16)> {
    let mut y = origin;
    distribute(weights, height as usize)
        .into_iter()
        .map(|rows| {
            let range = (y, rows as u16);
            y = y.saturating_add(rows as u16);
            range
        })
        .collect()
}

fn stage_by_rank(rank: u8) -> Stage {
    Stage::ALL
        .into_iter()
        .find(|stage| stage.pipeline_rank() == rank)
        .unwrap_or(Stage::Applied)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{ApplicationId, EventId, StageEvent};
    use jiff::civil::Date;

    fn date(month: i8, day: i8) -> Date {
        Date::new(2026, month, day).unwrap()
    }

    fn history(id: i64, events: Vec<(i64, Stage, Date)>) -> Application {
        Application::new(
            ApplicationId::new(id),
            "Acme".into(),
            "Role".into(),
            events
                .into_iter()
                .map(|(event_id, stage, on)| StageEvent {
                    id: EventId::new(event_id),
                    stage,
                    occurred_on: on,
                })
                .collect(),
        )
    }

    fn node(flow: &Flow, step: usize, stage: Stage) -> usize {
        flow.nodes
            .iter()
            .find(|node| node.step == step && node.stage == stage)
            .map(|node| node.count)
            .unwrap_or(0)
    }

    fn link(flow: &Flow, from_step: usize, from: Stage, to: Stage) -> usize {
        flow.links
            .iter()
            .find(|link| {
                link.from_step == from_step && link.from_stage == from && link.to_stage == to
            })
            .map(|link| link.count)
            .unwrap_or(0)
    }

    #[test]
    fn two_second_steps_and_a_stopped_application() {
        let flow = flow(&[
            history(
                1,
                vec![
                    (1, Stage::Applied, date(11, 1)),
                    (2, Stage::FirstMeeting, date(11, 2)),
                ],
            ),
            history(
                2,
                vec![
                    (3, Stage::Applied, date(11, 1)),
                    (4, Stage::Refused, date(11, 3)),
                ],
            ),
            history(3, vec![(5, Stage::Applied, date(11, 4))]),
        ]);

        assert_eq!(flow.applications, 3);
        assert_eq!(flow.steps, 2);
        assert_eq!(node(&flow, 0, Stage::Applied), 3);
        assert_eq!(node(&flow, 1, Stage::FirstMeeting), 1);
        assert_eq!(node(&flow, 1, Stage::Refused), 1);
        assert_eq!(link(&flow, 0, Stage::Applied, Stage::FirstMeeting), 1);
        assert_eq!(link(&flow, 0, Stage::Applied, Stage::Refused), 1);
        assert_eq!(flow.links.len(), 2);
    }

    #[test]
    fn a_repeated_stage_links_to_the_next_step() {
        let flow = flow(&[history(
            1,
            vec![
                (1, Stage::Applied, date(11, 1)),
                (2, Stage::Applied, date(11, 2)),
            ],
        )]);
        assert_eq!(link(&flow, 0, Stage::Applied, Stage::Applied), 1);
        assert_eq!(node(&flow, 0, Stage::Applied), 1);
        assert_eq!(node(&flow, 1, Stage::Applied), 1);
    }

    #[test]
    fn an_earlier_date_is_the_first_step_even_when_it_is_listed_later() {
        let flow = flow(&[history(
            1,
            vec![
                (2, Stage::Applied, date(11, 2)),
                (1, Stage::Refused, date(11, 1)),
            ],
        )]);
        assert_eq!(node(&flow, 0, Stage::Refused), 1);
        assert_eq!(node(&flow, 1, Stage::Applied), 1);
        assert_eq!(link(&flow, 0, Stage::Refused, Stage::Applied), 1);
        assert_eq!(node(&flow, 0, Stage::Applied), 0);
    }

    #[test]
    fn node_heights_fill_the_rows_above_the_footer() {
        let flow = flow(&[
            history(1, vec![(1, Stage::Applied, date(11, 1))]),
            history(2, vec![(2, Stage::Applied, date(11, 1))]),
            history(3, vec![(3, Stage::Applied, date(11, 1))]),
            history(4, vec![(4, Stage::Refused, date(11, 1))]),
        ]);
        let Placement::Chart(chart) = layout(&flow, 40, 12) else {
            panic!("expected a chart");
        };
        let heights: Vec<u16> = chart.nodes.iter().map(|node| node.height).collect();
        assert_eq!(heights, vec![6, 2]);
        let gaps = chart.nodes.len().saturating_sub(1) as u16;
        assert_eq!(
            heights.iter().sum::<u16>() + gaps,
            12 - 3,
            "node rows plus gaps fill the area above the footer"
        );
    }

    #[test]
    fn a_narrow_width_keeps_the_first_columns() {
        let flow = flow(&[history(
            1,
            vec![
                (1, Stage::Applied, date(11, 1)),
                (2, Stage::Refused, date(11, 2)),
            ],
        )]);
        let Placement::Chart(chart) = layout(&flow, 20, 12) else {
            panic!("expected a chart");
        };
        assert!(chart.nodes.iter().all(|node| node.step == 0));
        assert!(chart.ribbons.is_empty());
        assert!(
            chart.title.contains("showing first 1 of 2 steps"),
            "{}",
            chart.title
        );
    }

    #[test]
    fn a_tiny_height_is_too_small() {
        let flow = flow(&[
            history(1, vec![(1, Stage::Applied, date(11, 1))]),
            history(2, vec![(2, Stage::Refused, date(11, 1))]),
        ]);
        assert_eq!(layout(&flow, 40, 5), Placement::TooSmall);
    }

    #[test]
    fn a_continuing_application_places_one_ribbon() {
        let flow = flow(&[history(
            1,
            vec![
                (1, Stage::Applied, date(11, 1)),
                (2, Stage::Refused, date(11, 2)),
            ],
        )]);
        let Placement::Chart(chart) = layout(&flow, 80, 24) else {
            panic!("expected a chart");
        };
        assert_eq!(chart.ribbons.len(), 1);
        assert_eq!(chart.ribbons[0].stage, Stage::Applied);
        assert!(chart.ribbons[0].src_height > 0);
        assert!(chart.ribbons[0].dst_height > 0);
        assert!(
            chart.ribbons[0].width > GUTTER,
            "spare width widens the ribbon, got {}",
            chart.ribbons[0].width
        );
    }

    #[test]
    fn a_smaller_later_column_stays_shorter_than_the_screen() {
        let mut apps = Vec::new();
        for id in 1..=4 {
            apps.push(history(id, vec![(id, Stage::Applied, date(11, 1))]));
        }
        apps.push(history(
            5,
            vec![
                (5, Stage::Applied, date(11, 1)),
                (6, Stage::Refused, date(11, 2)),
            ],
        ));
        let Placement::Chart(chart) = layout(&flow(&apps), 80, 24) else {
            panic!("expected a chart");
        };
        let applied = chart
            .nodes
            .iter()
            .find(|node| node.step == 0 && node.stage == Stage::Applied)
            .unwrap();
        let refused = chart
            .nodes
            .iter()
            .find(|node| node.step == 1 && node.stage == Stage::Refused)
            .unwrap();
        assert_eq!(applied.height, 21);
        assert_eq!(refused.height, 4);
        assert!(refused.height < applied.height);
    }
}
