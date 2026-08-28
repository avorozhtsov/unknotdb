#!/usr/bin/env python3
"""Extract the lower-right orthogonal knot diagram from Wang--Zhang EPS.

This deliberately consumes vector paths, not pixels.  The published diagram
uses a gap in the under-strand at every crossing.  Black and magenta strokes
are two highlighted portions of the same knot.
"""

from __future__ import annotations

import argparse
import json
import re
from collections import defaultdict
from itertools import pairwise
from pathlib import Path
from typing import NamedTuple

NUMBER = re.compile(r"[-+]?(?:\d+\.\d*|\.\d+|\d+)|[A-Za-z*?]+")
BLACK = ("gray", 0.0)
MAGENTA = ("rgb", 1.0, 0.0, 1.0)
EPSILON = 0.03
RED_CIRCLES = (
    (188.211, 136.250),
    (200.098, 152.105),
    (219.918, 199.664),
    (237.750, 215.520),
    (271.441, 199.664),
)


class CrossingGeometry(NamedTuple):
    under_axis: str
    x: float
    y: float


def parse_segments(path: Path):
    page = path.read_text().split("%%EndPageSetup", 1)[1]
    stack: list[float] = []
    current = None
    color = BLACK
    segments = []
    for token in NUMBER.findall(page):
        try:
            stack.append(float(token))
            continue
        except ValueError:
            pass
        if token == "m":
            y, x = stack.pop(), stack.pop()
            current = (x, y)
        elif token == "l":
            y, x = stack.pop(), stack.pop()
            target = (x, y)
            segments.append((color, current, target))
            current = target
        elif token == "c":
            values = stack[-6:]
            del stack[-6:]
            current = (values[-2], values[-1])
        elif token == "g":
            color = ("gray", stack.pop())
        elif token == "rg":
            blue, green, red = stack.pop(), stack.pop(), stack.pop()
            color = ("rgb", red, green, blue)
        elif token == "cm":
            del stack[-6:]
        elif token in {"w", "J", "j", "M"}:
            stack.pop()
        elif token in {"d", "S", "f", "n", "q", "Q", "h", "W", "rectclip", "re"}:
            stack.clear()
    return segments


def merge_intervals(intervals):
    merged = []
    for start, end in sorted(intervals):
        if merged and start <= merged[-1][1] + 0.02:
            merged[-1] = (merged[-1][0], max(merged[-1][1], end))
        else:
            merged.append((start, end))
    return merged


def close(value: float, other: float) -> bool:
    return abs(value - other) < EPSILON


def interval_contains(interval, value: float) -> bool:
    return interval[0] - EPSILON < value < interval[1] + EPSILON


def reconstruct_topology(horizontal, vertical, crossings):
    """Return the four geometric ports and pairings between crossings.

    Axis-specific nodes keep the over- and under-strands disjoint at a
    crossing.  At every other shared endpoint they are identified, which
    recovers the corners of the orthogonal drawing.
    """
    horizontal = {level: list(items) for level, items in horizontal.items()}
    vertical = {level: list(items) for level, items in vertical.items()}
    for crossing in crossings:
        lines = horizontal if crossing.under_axis == "horizontal" else vertical
        level = crossing.y if crossing.under_axis == "horizontal" else crossing.x
        variable = crossing.x if crossing.under_axis == "horizontal" else crossing.y
        intervals = lines[level]
        if any(interval_contains(interval, variable) for interval in intervals):
            continue
        for index, (left, right) in enumerate(intervals[:-1]):
            next_left, next_right = intervals[index + 1]
            if right - EPSILON < variable < next_left + EPSILON:
                intervals[index : index + 2] = [(left, next_right)]
                break
        else:
            raise ValueError(f"cannot bridge under-gap at {crossing}")

    crossing_at = {
        (round(crossing.x, 3), round(crossing.y, 3)): crossing
        for crossing in crossings
    }
    endpoints_h = {
        (round(endpoint, 3), round(level, 3))
        for level, intervals in horizontal.items()
        for interval in intervals
        for endpoint in interval
    }
    endpoints_v = {
        (round(level, 3), round(endpoint, 3))
        for level, intervals in vertical.items()
        for interval in intervals
        for endpoint in interval
    }
    corners = (endpoints_h & endpoints_v) - set(crossing_at)

    graph = defaultdict(set)

    def add_edge(left, right):
        graph[left].add(right)
        graph[right].add(left)

    for axis, lines in (("h", horizontal), ("v", vertical)):
        for level, intervals in lines.items():
            for start, end in intervals:
                points = {start, end}
                for x, y in corners | set(crossing_at):
                    fixed, variable = (y, x) if axis == "h" else (x, y)
                    if close(fixed, level) and interval_contains((start, end), variable):
                        points.add(variable)
                ordered = sorted(points)
                for left, right in pairwise(ordered):
                    left_xy = (left, level) if axis == "h" else (level, left)
                    right_xy = (right, level) if axis == "h" else (level, right)
                    left_key = (axis, round(left_xy[0], 3), round(left_xy[1], 3))
                    right_key = (axis, round(right_xy[0], 3), round(right_xy[1], 3))
                    add_edge(left_key, right_key)

    # A corner is a single topological point.  Join the axis-specific nodes.
    for x, y in corners:
        add_edge(("h", x, y), ("v", x, y))

    crossing_nodes = set()
    for x, y in crossing_at:
        crossing_nodes.add(("h", x, y))
        crossing_nodes.add(("v", x, y))
    bad_degrees = {
        node: len(neighbors)
        for node, neighbors in graph.items()
        if node not in crossing_nodes and len(neighbors) != 2
    }
    crossing_degrees = {
        node: len(graph[node]) for node in crossing_nodes if len(graph[node]) != 2
    }
    if bad_degrees or crossing_degrees:
        raise ValueError(
            f"invalid orthogonal topology: ordinary={bad_degrees}, "
            f"crossings={crossing_degrees}"
        )

    def direction(source, target):
        _, x, y = source
        _, other_x, other_y = target
        if other_x > x + EPSILON:
            return "E"
        if other_x < x - EPSILON:
            return "W"
        if other_y > y + EPSILON:
            return "N"
        if other_y < y - EPSILON:
            return "S"
        raise ValueError(f"zero-length geometric edge {source} -> {target}")

    ports = {}
    pairings = []
    for crossing_id, crossing in enumerate(crossings):
        x, y = round(crossing.x, 3), round(crossing.y, 3)
        for axis in ("h", "v"):
            source = (axis, x, y)
            for neighbor in graph[source]:
                first_direction = direction(source, neighbor)
                previous, current = source, neighbor
                while current not in crossing_nodes:
                    next_nodes = graph[current] - {previous}
                    if len(next_nodes) != 1:
                        raise ValueError(f"ambiguous trace at {current}: {next_nodes}")
                    previous, current = current, next(iter(next_nodes))
                target_direction = direction(current, previous)
                target_xy = (current[1], current[2])
                target_crossing = crossings.index(crossing_at[target_xy])
                source_port = (crossing_id, first_direction)
                target_port = (target_crossing, target_direction)
                ports[source_port] = target_port
                if source_port < target_port:
                    pairings.append((source_port, target_port))
    return graph, ports, pairings


def spherogram_analysis(crossings, pairings):
    try:
        from spherogram import Crossing, Link
        from spherogram.links import seifert
    except ImportError as error:
        raise SystemExit(
            "spherogram is required for --spherogram; run with the RF Knots "
            "bounds environment"
        ) from error

    objects = [Crossing(index) for index in range(len(crossings))]

    def local_index(crossing, direction):
        if crossing.under_axis == "horizontal":
            return {"E": 0, "N": 1, "W": 2, "S": 3}[direction]
        return {"N": 0, "W": 1, "S": 2, "E": 3}[direction]

    for (left_id, left_direction), (right_id, right_direction) in pairings:
        left = local_index(crossings[left_id], left_direction)
        right = local_index(crossings[right_id], right_direction)
        objects[left_id][left] = objects[right_id][right]
    link = Link(objects, check_planarity=False)
    marked = []
    for red_x, red_y in RED_CIRCLES:
        nearest = min(
            range(len(crossings)),
            key=lambda index: abs(crossings[index].x - red_x)
            + abs(crossings[index].y - red_y),
        )
        distance = abs(crossings[nearest].x - red_x) + abs(crossings[nearest].y - red_y)
        if distance > 0.05:
            raise ValueError(f"red circle {(red_x, red_y)} has no crossing")
        marked.append(nearest)
    source_pd = link.PD_code()
    changed_pd = list(source_pd)
    for crossing_id in marked:
        crossing = changed_pd[crossing_id]
        changed_pd[crossing_id] = crossing[1:] + crossing[:1]
    changed = Link(changed_pd)
    simplified = changed.copy()
    simplify_error = None
    try:
        simplified.simplify("global")
    except Exception as error:  # noqa: BLE001 - retained in extraction audit
        simplify_error = f"{type(error).__name__}: {error}"

    # Run Vogel braidification once on the source diagram.  Original integer
    # crossing labels survive the added RII pairs, so the five published
    # crossings become five exact letter positions in one common braid word.
    braided = link.copy()
    seifert.isotope_to_braid(braided)
    circles = seifert.seifert_circles(braided)
    tree = seifert.seifert_tree(braided)
    tails = [edge[0] for edge in tree]
    heads = [edge[1] for edge in tree]
    start = next(index for index, tail in enumerate(tails) if tail not in heads)
    ordered_strands = [circles[start]]
    for _ in range(len(circles) - 1):
        new_tail = tree[start][1]
        start = tails.index(new_tail)
        ordered_strands.append(circles[start])
    positions_in_next_strand = []
    for index in range(len(ordered_strands) - 1):
        for cep_index, cep in enumerate(ordered_strands[index]):
            found = False
            for next_index, next_cep in enumerate(ordered_strands[index + 1]):
                if cep.crossing == next_cep.crossing:
                    ordered_strands[index + 1] = seifert.cyclic_permute(
                        ordered_strands[index + 1], next_index
                    )
                    found = True
                    break
            if found:
                break
    for index in range(len(ordered_strands) - 1):
        positions = {}
        for cep_index, cep in enumerate(ordered_strands[index]):
            for next_index, next_cep in enumerate(ordered_strands[index + 1]):
                if cep.crossing == next_cep.crossing:
                    positions[cep_index] = (
                        next_index,
                        cep.strand_index % 2,
                        cep.crossing.label,
                    )
                    break
        positions_in_next_strand.append(positions)
    ordered_strands.reverse()
    arrows = [
        [position, values[0], strand, values[1], values[2]]
        for strand, positions in enumerate(positions_in_next_strand)
        for position, values in positions.items()
    ]
    seifert.straighten_arrows(arrows)
    arrows.sort(key=lambda arrow: arrow[0])
    for arrow in arrows:
        arrow.pop(1)
    braid_word = [
        strand + 1 if over_or_under != 0 else -strand - 1
        for _, strand, over_or_under, _ in arrows
    ]
    marked_set = set(marked)
    marked_positions = [
        position
        for position, arrow in enumerate(arrows)
        if arrow[3] in marked_set
    ]
    if len(marked_positions) != len(RED_CIRCLES):
        raise ValueError(
            f"marked crossings did not survive braidification: {marked_positions}"
        )
    changed_braid = list(braid_word)
    for position in marked_positions:
        changed_braid[position] = -changed_braid[position]
    changed_braid_link = Link(braid_closure=changed_braid)
    changed_braid_link.simplify("global")
    return {
        "components": len(link.link_components),
        "pd_code": source_pd,
        "dt_code": link.DT_code(),
        "marked_crossing_ids": marked,
        "changed_components": len(changed.link_components),
        "changed_simplified_crossings": len(simplified.crossings),
        "changed_simplify_error": simplify_error,
        "braid_strands": max(map(abs, braid_word)) + 1,
        "braid_word": braid_word,
        "braid_crossing_labels": [arrow[3] for arrow in arrows],
        "marked_braid_positions": marked_positions,
        "changed_braid_simplified_crossings": len(changed_braid_link.crossings),
    }


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("eps", type=Path)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--spherogram", action="store_true")
    args = parser.parse_args()
    horizontal = defaultdict(list)
    vertical = defaultdict(list)
    selected = []
    for color, start, end in parse_segments(args.eps):
        if color not in {BLACK, MAGENTA}:
            continue
        if min(start[0], end[0]) < 167 or min(start[1], end[1]) < 127:
            continue
        if max(start[0], end[0]) > 304 or max(start[1], end[1]) > 232:
            continue
        selected.append((color, start, end))
        if abs(start[1] - end[1]) < 0.01:
            horizontal[round(start[1], 3)].append(tuple(sorted((start[0], end[0]))))
        elif abs(start[0] - end[0]) < 0.01:
            vertical[round(start[0], 3)].append(tuple(sorted((start[1], end[1]))))
    horizontal = {level: merge_intervals(items) for level, items in horizontal.items()}
    vertical = {level: merge_intervals(items) for level, items in vertical.items()}
    gaps = []
    for axis, lines, perpendicular in (
        ("horizontal", horizontal, vertical),
        ("vertical", vertical, horizontal),
    ):
        for level, intervals in lines.items():
            for (_, left), (right, _) in pairwise(intervals):
                distance = right - left
                crossing_levels = [
                    value
                    for value, perpendicular_intervals in perpendicular.items()
                    if left - EPSILON < value < right + EPSILON
                    and any(
                        start + 0.05 < level < end - 0.05
                        for start, end in perpendicular_intervals
                    )
                ]
                for crossing_level in crossing_levels:
                    gaps.append(
                        {
                            "under_axis": axis,
                            "x": crossing_level if axis == "horizontal" else level,
                            "y": level if axis == "horizontal" else crossing_level,
                            "gap": distance,
                            "perpendicular_continuous": True,
                        }
                    )
    crossings = [
        CrossingGeometry(gap["under_axis"], gap["x"], gap["y"])
        for gap in gaps
        if gap["perpendicular_continuous"]
        and gap["gap"] < 10.0
    ]
    graph, _ports, pairings = reconstruct_topology(horizontal, vertical, crossings)
    result = {
        "format": "wang-zhang-orthogonal-grid-extraction-v0",
        "source": str(args.eps),
        "selected_segments": len(selected),
        "horizontal_levels": len(horizontal),
        "vertical_levels": len(vertical),
        "candidate_gaps": gaps,
        "crossing_count": len(crossings),
        "topology_nodes": len(graph),
        "topology_pairings": len(pairings),
    }
    if args.spherogram:
        result["spherogram"] = spherogram_analysis(crossings, pairings)
    encoded = json.dumps(result, indent=2, sort_keys=True) + "\n"
    if args.output:
        args.output.write_text(encoded)
    else:
        print(encoded, end="")


if __name__ == "__main__":
    main()
