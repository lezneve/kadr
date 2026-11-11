//! the annotation model of the editor, without any windows or drawing code.
//!
//! the captured image is never touched: annotations are a list of shapes
//! drawn over it, in physical image pixels (one unit = one captured pixel,
//! whatever the dpi). every change is an edit on a history stack, so undo and
//! redo replay edits instead of keeping copies of the image. the history is
//! bounded (MAX_HISTORY edits, MAX_POINTS per freehand stroke).
//!
//! `Style` travels with each shape, so colour and width (stage 4) only need a
//! way to set the current style; nothing here changes.

pub const MAX_HISTORY: usize = 500;
pub const MAX_POINTS: usize = 20_000;

#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Pt {
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Area {
    pub l: f32,
    pub t: f32,
    pub r: f32,
    pub b: f32,
}

impl Area {
    pub fn from_points(a: Pt, b: Pt) -> Area {
        Area { l: a.x.min(b.x), t: a.y.min(b.y), r: a.x.max(b.x), b: a.y.max(b.y) }
    }
    pub fn inflate(self, d: f32) -> Area {
        Area { l: self.l - d, t: self.t - d, r: self.r + d, b: self.b + d }
    }
    pub fn union(self, o: Area) -> Area {
        Area { l: self.l.min(o.l), t: self.t.min(o.t), r: self.r.max(o.r), b: self.b.max(o.b) }
    }
    pub fn intersects(self, o: Area) -> bool {
        self.l < o.r && o.l < self.r && self.t < o.b && o.t < self.b
    }
    pub fn contains(self, p: Pt) -> bool {
        p.x >= self.l && p.x <= self.r && p.y >= self.t && p.y <= self.b
    }
    pub fn width(self) -> f32 {
        self.r - self.l
    }
    pub fn height(self) -> f32 {
        self.b - self.t
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Style {
    /// straight (not premultiplied) rgba, 0..1
    pub color: [f32; 4],
    /// stroke width in image pixels
    pub width: f32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tool {
    Select,
    Pen,
    Line,
    Rect,
    Ellipse,
    Highlighter,
    Blur,
    Redact,
}

impl Tool {
    pub const ALL: [Tool; 8] = [Tool::Select, Tool::Pen, Tool::Line, Tool::Rect, Tool::Ellipse, Tool::Highlighter, Tool::Blur, Tool::Redact];

    pub fn label(self) -> &'static str {
        match self {
            Tool::Select => "select · v",
            Tool::Pen => "pen · p",
            Tool::Line => "line · l",
            Tool::Rect => "rectangle · r",
            Tool::Ellipse => "ellipse · e",
            Tool::Highlighter => "highlighter · h",
            Tool::Blur => "blur · b · not secure, use redact",
            Tool::Redact => "redact · x",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Tool::Select => "select",
            Tool::Pen => "pen",
            Tool::Line => "line",
            Tool::Rect => "rect",
            Tool::Ellipse => "ellipse",
            Tool::Highlighter => "highlighter",
            Tool::Blur => "blur",
            Tool::Redact => "redact",
        }
    }

    /// which style this tool draws with; none for select and blur.
    pub fn slot(self) -> Option<Slot> {
        match self {
            Tool::Pen | Tool::Line | Tool::Rect | Tool::Ellipse => Some(Slot::Stroke),
            Tool::Highlighter => Some(Slot::Highlight),
            Tool::Redact => Some(Slot::Redact),
            Tool::Select | Tool::Blur => None,
        }
    }
}

/// the separate style settings: strokes (pen, line, rectangle, ellipse)
/// share one; the highlighter keeps its own width and a fixed 40 % opacity;
/// redact has a colour only. blur has none.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Slot {
    Stroke,
    Highlight,
    Redact,
}

impl Slot {
    pub fn name(self) -> &'static str {
        match self {
            Slot::Stroke => "stroke",
            Slot::Highlight => "highlighter",
            Slot::Redact => "redact",
        }
    }
    pub fn has_width(self) -> bool {
        self != Slot::Redact
    }
    /// the opacity this slot always draws with
    pub fn alpha(self) -> f32 {
        if self == Slot::Highlight { HIGHLIGHT_ALPHA } else { 1.0 }
    }
}

pub const HIGHLIGHT_ALPHA: f32 = 0.4;
pub const MIN_WIDTH: f32 = 1.0;
pub const MAX_WIDTH: f32 = 32.0;

/// the preset colours of the style panel (rgb).
pub const PRESETS: [u32; 7] = [0xff3b30, 0xff9500, 0xffd60a, 0x34c759, 0x0a84ff, 0x000000, 0xffffff];

pub fn rgb_of(hex: u32) -> [f32; 3] {
    [((hex >> 16) & 0xff) as f32 / 255.0, ((hex >> 8) & 0xff) as f32 / 255.0, (hex & 0xff) as f32 / 255.0]
}

pub fn hex_of(c: [f32; 4]) -> u32 {
    let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u32;
    (b(c[0]) << 16) | (b(c[1]) << 8) | b(c[2])
}

/// the current style of each slot; remembered between editors (editor.ini).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Styles {
    pub stroke: Style,
    pub highlight: Style,
    pub redact: Style,
}

impl Default for Styles {
    fn default() -> Self {
        Styles {
            stroke: Style { color: [1.0, 0.23, 0.19, 1.0], width: 4.0 },
            highlight: Style { color: [1.0, 0.84, 0.04, HIGHLIGHT_ALPHA], width: 18.0 },
            redact: Style { color: [0.0, 0.0, 0.0, 1.0], width: 0.0 },
        }
    }
}

impl Styles {
    pub fn get(&self, slot: Slot) -> Style {
        match slot {
            Slot::Stroke => self.stroke,
            Slot::Highlight => self.highlight,
            Slot::Redact => self.redact,
        }
    }

    /// the style as `slot` stores it: its own opacity, width clamped (or none).
    pub fn normalized(slot: Slot, style: Style) -> Style {
        Style {
            color: [style.color[0], style.color[1], style.color[2], slot.alpha()],
            width: if slot.has_width() { style.width.round().clamp(MIN_WIDTH, MAX_WIDTH) } else { 0.0 },
        }
    }

    pub fn set(&mut self, slot: Slot, style: Style) {
        let s = Self::normalized(slot, style);
        match slot {
            Slot::Stroke => self.stroke = s,
            Slot::Highlight => self.highlight = s,
            Slot::Redact => self.redact = s,
        }
    }

    /// the style a new shape of `tool` gets.
    pub fn for_tool(&self, tool: Tool) -> Style {
        match tool.slot() {
            Some(slot) => self.get(slot),
            None => Style { color: [0.0, 0.0, 0.0, 0.0], width: 0.0 },
        }
    }

    /// `stroke = #ff3b30 4` lines; anything unreadable keeps its default.
    pub fn parse(text: &str) -> Styles {
        let mut out = Styles::default();
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else { continue };
            let slot = match key.trim() {
                "stroke" => Slot::Stroke,
                "highlighter" => Slot::Highlight,
                "redact" => Slot::Redact,
                _ => continue,
            };
            let mut parts = value.split_whitespace();
            let Some(hex) = parts.next().and_then(|h| h.strip_prefix('#')).filter(|h| h.len() == 6).and_then(|h| u32::from_str_radix(h, 16).ok()) else { continue };
            let width = parts.next().and_then(|w| w.parse::<f32>().ok()).unwrap_or(out.get(slot).width);
            let c = rgb_of(hex);
            out.set(slot, Style { color: [c[0], c[1], c[2], 1.0], width });
        }
        out
    }

    pub fn to_text(&self) -> String {
        let line = |slot: Slot| {
            let s = self.get(slot);
            if slot.has_width() {
                format!("{} = #{:06x} {}", slot.name(), hex_of(s.color), s.width.round() as u32)
            } else {
                format!("{} = #{:06x}", slot.name(), hex_of(s.color))
            }
        };
        let lines = ["# kadr editor: the last colours and widths. written by the editor.".to_string(), line(Slot::Stroke), line(Slot::Highlight), line(Slot::Redact)];
        lines.join("\r\n") + "\r\n"
    }
}

#[derive(Clone, PartialEq, Debug)]
pub enum Shape {
    Pen(Vec<Pt>),
    Highlight(Vec<Pt>),
    Line(Pt, Pt),
    Rect(Area),
    Ellipse(Area),
    /// gaussian blur of the source image under the area. not secure.
    Blur(Area),
    /// opaque fill: the pixels under it are gone from the export.
    Redact(Area),
}

impl Shape {
    /// the style slot this shape belongs to (none for blur).
    pub fn slot(&self) -> Option<Slot> {
        match self {
            Shape::Pen(_) | Shape::Line(..) | Shape::Rect(_) | Shape::Ellipse(_) => Some(Slot::Stroke),
            Shape::Highlight(_) => Some(Slot::Highlight),
            Shape::Redact(_) => Some(Slot::Redact),
            Shape::Blur(_) => None,
        }
    }

    pub fn kind(&self) -> &'static str {
        match self {
            Shape::Pen(_) => "pen",
            Shape::Highlight(_) => "highlighter",
            Shape::Line(..) => "line",
            Shape::Rect(_) => "rect",
            Shape::Ellipse(_) => "ellipse",
            Shape::Blur(_) => "blur",
            Shape::Redact(_) => "redact",
        }
    }

    /// the geometric bounds, without stroke width.
    pub fn bounds(&self) -> Area {
        match self {
            Shape::Pen(p) | Shape::Highlight(p) => {
                let first = p.first().copied().unwrap_or_default();
                p.iter().fold(Area::from_points(first, first), |a, q| a.union(Area::from_points(*q, *q)))
            }
            Shape::Line(a, b) => Area::from_points(*a, *b),
            Shape::Rect(r) | Shape::Ellipse(r) | Shape::Blur(r) | Shape::Redact(r) => *r,
        }
    }

    /// everything the shape can paint, stroke included.
    pub fn extent(&self, style: &Style) -> Area {
        self.bounds().inflate(style.width / 2.0 + 2.0)
    }

    pub fn translated(&self, dx: f32, dy: f32) -> Shape {
        let m = |p: &Pt| Pt { x: p.x + dx, y: p.y + dy };
        let a = |r: &Area| Area { l: r.l + dx, t: r.t + dy, r: r.r + dx, b: r.b + dy };
        match self {
            Shape::Pen(p) => Shape::Pen(p.iter().map(m).collect()),
            Shape::Highlight(p) => Shape::Highlight(p.iter().map(m).collect()),
            Shape::Line(p, q) => Shape::Line(m(p), m(q)),
            Shape::Rect(r) => Shape::Rect(a(r)),
            Shape::Ellipse(r) => Shape::Ellipse(a(r)),
            Shape::Blur(r) => Shape::Blur(a(r)),
            Shape::Redact(r) => Shape::Redact(a(r)),
        }
    }

    /// resize handles: four corners for area shapes, two ends for a line,
    /// none for freehand strokes (they move only).
    pub fn handles(&self) -> Vec<Pt> {
        match self {
            Shape::Line(a, b) => vec![*a, *b],
            Shape::Rect(r) | Shape::Ellipse(r) | Shape::Blur(r) | Shape::Redact(r) => {
                vec![Pt { x: r.l, y: r.t }, Pt { x: r.r, y: r.t }, Pt { x: r.r, y: r.b }, Pt { x: r.l, y: r.b }]
            }
            Shape::Pen(_) | Shape::Highlight(_) => Vec::new(),
        }
    }

    /// the shape with handle `i` dragged to `p`; `square` keeps squares and
    /// circles (shift) or snaps a line to 45°.
    pub fn with_handle(&self, i: usize, p: Pt, square: bool) -> Shape {
        match self {
            Shape::Line(a, b) => {
                if i == 0 { Shape::Line(constrain_line(*b, p, square), *b) } else { Shape::Line(*a, constrain_line(*a, p, square)) }
            }
            Shape::Rect(r) | Shape::Ellipse(r) | Shape::Blur(r) | Shape::Redact(r) => {
                // the corner opposite the handle stays put
                let fixed = match i {
                    0 => Pt { x: r.r, y: r.b },
                    1 => Pt { x: r.l, y: r.b },
                    2 => Pt { x: r.l, y: r.t },
                    _ => Pt { x: r.r, y: r.t },
                };
                self.with_area(constrain_area(fixed, p, square))
            }
            other => other.clone(),
        }
    }

    fn with_area(&self, a: Area) -> Shape {
        match self {
            Shape::Rect(_) => Shape::Rect(a),
            Shape::Ellipse(_) => Shape::Ellipse(a),
            Shape::Blur(_) => Shape::Blur(a),
            Shape::Redact(_) => Shape::Redact(a),
            other => other.clone(),
        }
    }

    /// a new shape being drawn from `a` to `b`.
    pub fn drawn(tool: Tool, a: Pt, b: Pt, square: bool) -> Option<Shape> {
        let area = constrain_area(a, b, square);
        Some(match tool {
            Tool::Line => Shape::Line(a, constrain_line(a, b, square)),
            Tool::Rect => Shape::Rect(area),
            Tool::Ellipse => Shape::Ellipse(area),
            Tool::Blur => Shape::Blur(area),
            Tool::Redact => Shape::Redact(area),
            Tool::Pen => Shape::Pen(vec![a, b]),
            Tool::Highlighter => Shape::Highlight(vec![a, b]),
            Tool::Select => return None,
        })
    }

    /// too small to keep (a click without a drag).
    pub fn is_degenerate(&self) -> bool {
        let b = self.bounds();
        match self {
            Shape::Pen(p) | Shape::Highlight(p) => p.len() < 2,
            Shape::Line(..) => b.width() < 2.0 && b.height() < 2.0,
            _ => b.width() < 2.0 || b.height() < 2.0,
        }
    }

    /// true when `p` is on the shape, within `tol` pixels of what it paints.
    pub fn hit(&self, p: Pt, style: &Style, tol: f32) -> bool {
        let reach = style.width / 2.0 + tol;
        match self {
            Shape::Pen(pts) | Shape::Highlight(pts) => {
                if pts.len() == 1 {
                    return dist(p, pts[0]) <= reach;
                }
                pts.windows(2).any(|w| segment_dist(p, w[0], w[1]) <= reach)
            }
            Shape::Line(a, b) => segment_dist(p, *a, *b) <= reach,
            Shape::Rect(r) => {
                let outer = r.inflate(reach);
                let inner = r.inflate(-reach);
                outer.contains(p) && !(inner.width() > 0.0 && inner.height() > 0.0 && inner.contains(p))
            }
            Shape::Ellipse(r) => {
                let (cx, cy) = ((r.l + r.r) / 2.0, (r.t + r.b) / 2.0);
                let (rx, ry) = ((r.width() / 2.0).max(0.5), (r.height() / 2.0).max(0.5));
                let d = (((p.x - cx) / rx).powi(2) + ((p.y - cy) / ry).powi(2)).sqrt();
                (d - 1.0).abs() * rx.min(ry) <= reach
            }
            Shape::Blur(r) | Shape::Redact(r) => r.inflate(tol).contains(p),
        }
    }
}

pub fn dist(a: Pt, b: Pt) -> f32 {
    ((a.x - b.x).powi(2) + (a.y - b.y).powi(2)).sqrt()
}

fn segment_dist(p: Pt, a: Pt, b: Pt) -> f32 {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let len2 = dx * dx + dy * dy;
    if len2 == 0.0 {
        return dist(p, a);
    }
    let t = (((p.x - a.x) * dx + (p.y - a.y) * dy) / len2).clamp(0.0, 1.0);
    dist(p, Pt { x: a.x + t * dx, y: a.y + t * dy })
}

/// the area from a fixed corner `a` to `b`; with `square`, the larger side
/// wins and the square grows towards `b`.
pub fn constrain_area(a: Pt, b: Pt, square: bool) -> Area {
    if !square {
        return Area::from_points(a, b);
    }
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let side = dx.abs().max(dy.abs());
    let q = Pt { x: a.x + side * if dx < 0.0 { -1.0 } else { 1.0 }, y: a.y + side * if dy < 0.0 { -1.0 } else { 1.0 } };
    Area::from_points(a, q)
}

/// with `square`, snaps the line from `a` to multiples of 45°.
pub fn constrain_line(a: Pt, b: Pt, square: bool) -> Pt {
    if !square {
        return b;
    }
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let len = (dx * dx + dy * dy).sqrt();
    let angle = dy.atan2(dx);
    let step = std::f32::consts::FRAC_PI_4;
    let snapped = (angle / step).round() * step;
    // exact for the axes and diagonals
    let (c, s) = (snapped.cos(), snapped.sin());
    let r = |v: f32| if v.abs() < 1e-4 { 0.0 } else { v };
    let diag = c.abs() > 0.1 && s.abs() > 0.1;
    let l = if diag { len / std::f32::consts::SQRT_2 } else { len };
    if diag {
        Pt { x: a.x + l * c.signum(), y: a.y + l * s.signum() }
    } else {
        Pt { x: a.x + r(c) * l, y: a.y + r(s) * l }
    }
}

/// adds a freehand point, skipping ones closer than a pixel to the last and
/// stopping at MAX_POINTS. returns false when the point was not added.
pub fn push_point(points: &mut Vec<Pt>, p: Pt) -> bool {
    if points.len() >= MAX_POINTS {
        return false;
    }
    if points.last().is_some_and(|q| dist(*q, p) < 1.0) {
        return false;
    }
    points.push(p);
    true
}

#[derive(Clone, Debug)]
pub struct Item {
    pub id: u64,
    pub shape: Shape,
    pub style: Style,
}

#[derive(Clone, Debug)]
enum Edit {
    Add(usize, Item),
    Remove(usize, Item),
    Change { id: u64, before: Shape, after: Shape },
    Restyle { id: u64, before: Style, after: Style },
}

/// the annotations and their history.
#[derive(Default)]
pub struct Doc {
    pub items: Vec<Item>,
    /// (state id after the edit, edit). the state id names a document
    /// state, so "saved" survives undo back to it.
    undo: Vec<(u64, Edit)>,
    redo: Vec<(u64, Edit)>,
    next_id: u64,
    next_state: u64,
    /// the state below the oldest undoable edit once the history was capped:
    /// undoing everything left then is not the untouched capture.
    floor: u64,
    /// the shape whose last edit was a mergeable restyle (wheel steps)
    merging: Option<u64>,
}

impl Doc {
    /// the id of the current state; 0 = the untouched capture.
    pub fn state(&self) -> u64 {
        self.undo.last().map_or(self.floor, |(s, _)| *s)
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
    pub fn history_len(&self) -> usize {
        self.undo.len() + self.redo.len()
    }

    pub fn find(&self, id: u64) -> Option<&Item> {
        self.items.iter().find(|i| i.id == id)
    }

    fn record(&mut self, edit: Edit) {
        self.merging = None;
        self.next_state += 1;
        self.undo.push((self.next_state, edit));
        self.redo.clear();
        if self.undo.len() > MAX_HISTORY {
            // the oldest edits stop being undoable; the shapes stay
            let extra = self.undo.len() - MAX_HISTORY;
            self.floor = self.undo[extra - 1].0;
            self.undo.drain(..extra);
        }
    }

    pub fn add(&mut self, shape: Shape, style: Style) -> u64 {
        self.next_id += 1;
        let item = Item { id: self.next_id, shape, style };
        self.items.push(item.clone());
        self.record(Edit::Add(self.items.len() - 1, item));
        self.next_id
    }

    /// the area that changed.
    pub fn remove(&mut self, id: u64) -> Option<Area> {
        let index = self.items.iter().position(|i| i.id == id)?;
        let item = self.items.remove(index);
        let area = item.shape.extent(&item.style);
        self.record(Edit::Remove(index, item));
        Some(area)
    }

    /// records a finished move or resize; `before` is the shape when it began.
    pub fn change(&mut self, id: u64, before: Shape, after: Shape) {
        if before == after {
            return;
        }
        if let Some(i) = self.items.iter_mut().find(|i| i.id == id) {
            i.shape = after.clone();
        }
        self.record(Edit::Change { id, before, after });
    }

    /// records a new style for a shape. `merge`: fold into the previous edit
    /// when that was a restyle of the same shape (a slider drag or wheel turns
    /// are one undo step); the document still gets a new state id.
    pub fn restyle(&mut self, id: u64, before: Style, after: Style, merge: bool) -> Option<Area> {
        let item = self.items.iter_mut().find(|i| i.id == id)?;
        let area = item.shape.extent(&item.style).union(item.shape.extent(&after));
        item.style = after;
        if merge
            && self.merging == Some(id)
            && self.redo.is_empty()
            && let Some((state, Edit::Restyle { id: last, after: a, .. })) = self.undo.last_mut()
            && *last == id
        {
            *a = after;
            self.next_state += 1;
            *state = self.next_state;
            return Some(area);
        }
        if before != after {
            self.record(Edit::Restyle { id, before, after });
            if merge {
                self.merging = Some(id);
            }
        }
        Some(area)
    }

    /// shows a style while it is being chosen, without recording anything.
    pub fn preview_style(&mut self, id: u64, style: Style) -> Option<Area> {
        let item = self.items.iter_mut().find(|i| i.id == id)?;
        let area = item.shape.extent(&item.style).union(item.shape.extent(&style));
        item.style = style;
        Some(area)
    }

    /// shows a shape as it is being moved, without recording anything.
    pub fn preview(&mut self, id: u64, shape: Shape) {
        if let Some(i) = self.items.iter_mut().find(|i| i.id == id) {
            i.shape = shape;
        }
    }

    fn apply(&mut self, edit: &Edit, forward: bool) -> Area {
        match (edit, forward) {
            (Edit::Add(i, item), true) | (Edit::Remove(i, item), false) => {
                self.items.insert((*i).min(self.items.len()), item.clone());
                item.shape.extent(&item.style)
            }
            (Edit::Add(_, item), false) | (Edit::Remove(_, item), true) => {
                self.items.retain(|x| x.id != item.id);
                item.shape.extent(&item.style)
            }
            (Edit::Change { id, before, after }, fwd) => {
                let to = if fwd { after } else { before };
                let mut area = to.bounds();
                if let Some(i) = self.items.iter_mut().find(|i| i.id == *id) {
                    area = i.shape.extent(&i.style).union(to.extent(&i.style));
                    i.shape = to.clone();
                }
                area
            }
            (Edit::Restyle { id, before, after }, fwd) => {
                let to = if fwd { *after } else { *before };
                let mut area = Area::default();
                if let Some(i) = self.items.iter_mut().find(|i| i.id == *id) {
                    area = i.shape.extent(&i.style).union(i.shape.extent(&to));
                    i.style = to;
                }
                area
            }
        }
    }

    pub fn undo(&mut self) -> Option<Area> {
        self.merging = None;
        let (state, edit) = self.undo.pop()?;
        let area = self.apply(&edit, false);
        self.redo.push((state, edit));
        Some(area)
    }

    pub fn redo(&mut self) -> Option<Area> {
        self.merging = None;
        let (state, edit) = self.redo.pop()?;
        let area = self.apply(&edit, true);
        self.undo.push((state, edit));
        Some(area)
    }

    /// the topmost item at `p`.
    pub fn hit(&self, p: Pt, tol: f32) -> Option<u64> {
        self.items.iter().rev().find(|i| i.shape.hit(p, &i.style, tol)).map(|i| i.id)
    }

    /// rough memory held by shapes and history, for measurements.
    pub fn approx_bytes(&self) -> usize {
        let shape = |s: &Shape| match s {
            Shape::Pen(p) | Shape::Highlight(p) => p.len() * 8 + 64,
            _ => 64,
        };
        let edit = |e: &Edit| match e {
            Edit::Add(_, i) | Edit::Remove(_, i) => shape(&i.shape),
            Edit::Change { before, after, .. } => shape(before) + shape(after),
            Edit::Restyle { .. } => 48,
        };
        self.items.iter().map(|i| shape(&i.shape)).sum::<usize>() + self.undo.iter().chain(&self.redo).map(|(_, e)| edit(e)).sum::<usize>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pt(x: f32, y: f32) -> Pt {
        Pt { x, y }
    }

    #[test]
    fn undo_redo_and_saved_state() {
        let mut d = Doc::default();
        assert_eq!(d.state(), 0);
        let s = Styles::default().stroke;
        let a = d.add(Shape::Rect(Area { l: 0.0, t: 0.0, r: 10.0, b: 10.0 }), s);
        let after_add = d.state();
        let moved = d.find(a).unwrap().shape.translated(5.0, 0.0);
        d.change(a, d.find(a).unwrap().shape.clone(), moved.clone());
        assert_eq!(d.find(a).unwrap().shape, moved);
        let saved = d.state();
        d.undo();
        assert_eq!(d.state(), after_add);
        assert_eq!(d.find(a).unwrap().shape.bounds().l, 0.0);
        d.redo();
        assert_eq!(d.state(), saved, "redo returns to the saved state");
        d.remove(a);
        assert!(d.items.is_empty());
        d.undo();
        assert_eq!(d.items.len(), 1);
        d.undo();
        d.undo();
        assert_eq!(d.state(), 0, "undo all = the untouched capture");
        assert!(d.items.is_empty());
        // a new edit after undo drops the redo branch and gets a new state id
        d.add(Shape::Line(pt(0.0, 0.0), pt(5.0, 5.0)), s);
        assert!(!d.can_redo());
        assert_ne!(d.state(), saved);
        assert_ne!(d.state(), after_add);
    }

    #[test]
    fn history_is_bounded() {
        let mut d = Doc::default();
        for i in 0..(MAX_HISTORY + 50) {
            d.add(Shape::Rect(Area { l: i as f32, t: 0.0, r: i as f32 + 5.0, b: 5.0 }), Styles::default().stroke);
        }
        assert_eq!(d.history_len(), MAX_HISTORY);
        assert_eq!(d.items.len(), MAX_HISTORY + 50, "shapes stay, only undo depth is capped");
        while d.undo().is_some() {}
        assert_eq!(d.items.len(), 50, "the 50 oldest shapes cannot be undone");
        assert_ne!(d.state(), 0, "and the document is not the untouched capture");
        let mut points = Vec::new();
        for i in 0..(MAX_POINTS + 10) {
            push_point(&mut points, pt(i as f32 * 2.0, 0.0));
        }
        assert_eq!(points.len(), MAX_POINTS);
        let mut close = vec![pt(0.0, 0.0)];
        assert!(!push_point(&mut close, pt(0.5, 0.3)), "sub-pixel moves are skipped");
    }

    #[test]
    fn shift_constraints() {
        let sq = constrain_area(pt(10.0, 10.0), pt(40.0, 25.0), true);
        assert_eq!((sq.width(), sq.height()), (30.0, 30.0));
        let up_left = constrain_area(pt(10.0, 10.0), pt(0.0, -30.0), true);
        assert_eq!((up_left.l, up_left.t, up_left.r, up_left.b), (-30.0, -30.0, 10.0, 10.0));
        let h = constrain_line(pt(0.0, 0.0), pt(100.0, 7.0), true);
        assert!(h.y == 0.0 && (h.x - 100.245).abs() < 0.01, "horizontal keeps the length: {h:?}");
        let d = constrain_line(pt(0.0, 0.0), pt(50.0, 47.0), true);
        assert!((d.x - d.y).abs() < 1e-3, "45 degrees: {d:?}");
        assert_eq!(constrain_line(pt(0.0, 0.0), pt(3.0, -60.0), true).x, 0.0);
    }

    #[test]
    fn restyle_undo_redo_and_merge() {
        let mut d = Doc::default();
        let s = Styles::default();
        let id = d.add(Shape::Rect(Area { l: 0.0, t: 0.0, r: 10.0, b: 10.0 }), s.stroke);
        let blue = Style { color: [0.04, 0.52, 1.0, 1.0], width: 4.0 };
        d.restyle(id, s.stroke, blue, false);
        let wide = Style { width: 12.0, ..blue };
        let before_merge = d.history_len();
        let state_before = d.state();
        d.restyle(id, blue, Style { width: 6.0, ..blue }, true);
        d.restyle(id, blue, Style { width: 8.0, ..blue }, true);
        d.restyle(id, blue, wide, true);
        assert_eq!(d.history_len(), before_merge + 1, "wheel steps fold into one edit, not into the colour change before them");
        assert_ne!(d.state(), state_before, "a merged change is still a new state");
        assert_eq!(d.find(id).unwrap().style, wide);
        let after = d.state();
        d.undo();
        assert_eq!(d.find(id).unwrap().style, blue, "one undo takes back all wheel steps");
        d.undo();
        assert_eq!(d.find(id).unwrap().style, s.stroke, "the next one the colour");
        d.redo();
        d.redo();
        assert_eq!(d.find(id).unwrap().style, wide);
        assert_eq!(d.state(), after);
    }

    #[test]
    fn styles_round_trip_and_clamp() {
        let mut st = Styles::default();
        st.set(Slot::Stroke, Style { color: [0.0, 0.0, 1.0, 1.0], width: 99.0 });
        st.set(Slot::Highlight, Style { color: [0.0, 1.0, 0.0, 1.0], width: 0.2 });
        st.set(Slot::Redact, Style { color: [1.0, 1.0, 1.0, 1.0], width: 7.0 });
        assert_eq!(st.stroke.width, MAX_WIDTH);
        assert_eq!(st.highlight.width, MIN_WIDTH);
        assert_eq!(st.highlight.color[3], HIGHLIGHT_ALPHA, "the highlighter keeps its opacity");
        let text = st.to_text();
        assert!(text.contains("stroke = #0000ff 32") && text.contains("redact = #ffffff\r\n"), "{text}");
        assert_eq!(Styles::parse(&text), st);
        let bad = Styles::parse("stroke = red 4\nhighlighter = #12345678\nnonsense\nredact = #00ff00 3");
        assert_eq!(bad.stroke, Styles::default().stroke);
        assert_eq!(bad.highlight, Styles::default().highlight);
        assert_eq!(hex_of(bad.redact.color), 0x00ff00);
    }

    #[test]
    fn hits() {
        let s = Styles::default().stroke;
        let r = Shape::Rect(Area { l: 10.0, t: 10.0, r: 110.0, b: 60.0 });
        assert!(r.hit(pt(10.0, 30.0), &s, 4.0), "on the edge");
        assert!(!r.hit(pt(60.0, 35.0), &s, 4.0), "an outline is not hit in its middle");
        let e = Shape::Ellipse(Area { l: 0.0, t: 0.0, r: 100.0, b: 50.0 });
        assert!(e.hit(pt(50.0, 0.0), &s, 4.0) && !e.hit(pt(50.0, 25.0), &s, 4.0));
        let redact = Shape::Redact(Area { l: 0.0, t: 0.0, r: 20.0, b: 20.0 });
        assert!(redact.hit(pt(10.0, 10.0), &s, 4.0), "filled areas are hit inside");
        let line = Shape::Line(pt(0.0, 0.0), pt(100.0, 0.0));
        assert!(line.hit(pt(50.0, 3.0), &s, 4.0) && !line.hit(pt(50.0, 12.0), &s, 4.0));
        let mut d = Doc::default();
        let low = d.add(redact, s);
        let top = d.add(Shape::Redact(Area { l: 5.0, t: 5.0, r: 15.0, b: 15.0 }), s);
        assert_eq!(d.hit(pt(10.0, 10.0), 2.0), Some(top), "the topmost shape wins");
        assert_eq!(d.hit(pt(1.0, 1.0), 0.5), Some(low));
    }

    #[test]
    fn handles_resize_from_the_opposite_corner() {
        let r = Shape::Rect(Area { l: 10.0, t: 10.0, r: 50.0, b: 30.0 });
        let grown = r.with_handle(2, pt(70.0, 80.0), false);
        assert_eq!(grown.bounds(), Area { l: 10.0, t: 10.0, r: 70.0, b: 80.0 });
        let square = r.with_handle(2, pt(70.0, 40.0), true);
        assert_eq!((square.bounds().width(), square.bounds().height()), (60.0, 60.0));
        let line = Shape::Line(pt(0.0, 0.0), pt(10.0, 0.0));
        assert_eq!(line.with_handle(1, pt(20.0, 5.0), false), Shape::Line(pt(0.0, 0.0), pt(20.0, 5.0)));
        assert!(Shape::Pen(vec![pt(0.0, 0.0), pt(1.0, 1.0)]).handles().is_empty());
    }
}
