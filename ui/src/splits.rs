//! Basit bir ikili bölme (binary split) ağacı. Her yaprak bir `PaneId`
//! (bir `lumenterm_core::Session` ile eşleşir), her iç düğüm yatay/dikey
//! bir bölme, kararlı bir `id` (sürüklenebilir ayracı hit-test etmek
//! için) ve sürüklenebilir bir oran (`ratio: 0.0..1.0`) tutar.

pub type PaneId = u64;
pub type SplitId = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitDirection {
    Horizontal,
    Vertical,
}

#[derive(Debug, Clone)]
pub enum SplitNode {
    Leaf(PaneId),
    Split {
        id: SplitId,
        direction: SplitDirection,
        ratio: f32,
        first: Box<SplitNode>,
        second: Box<SplitNode>,
    },
}

pub struct SplitTree {
    pub root: SplitNode,
    pub active: PaneId,
    next_pane_id: PaneId,
    next_split_id: SplitId,
}

/// Bir pane'in ekrandaki piksel dikdörtgeni.
#[derive(Debug, Clone, Copy)]
pub struct PaneRect {
    pub pane: PaneId,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// Fare ile sürüklenebilir bir ayracın ekran dikdörtgeni (hit-test için
/// `divider_px` kadar genişletilmiş).
#[derive(Debug, Clone, Copy)]
pub struct DividerRect {
    pub split: SplitId,
    pub direction: SplitDirection,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// Bir split düğümünün (iki çocuğunu birlikte kapsayan) tam dikdörtgeni;
/// ayracı sürüklerken fare pozisyonundan yeni `ratio`'yu hesaplamak için.
#[derive(Debug, Clone, Copy)]
pub struct SplitRect {
    pub split: SplitId,
    pub direction: SplitDirection,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl SplitTree {
    pub fn new() -> Self {
        Self {
            root: SplitNode::Leaf(0),
            active: 0,
            next_pane_id: 1,
            next_split_id: 1,
        }
    }

    /// Aktif pane'i verilen yönde böler, yeni bir pane oluşturur ve onu
    /// aktif yapar. Yeni pane'in `Session`'ını kurmak `app`'in sorumluluğundadır.
    pub fn split_active(&mut self, direction: SplitDirection) -> PaneId {
        let new_pane = self.next_pane_id;
        self.next_pane_id += 1;
        let split_id = self.next_split_id;
        self.next_split_id += 1;
        let active = self.active;
        Self::replace_leaf(&mut self.root, active, |leaf_id| SplitNode::Split {
            id: split_id,
            direction,
            ratio: 0.5,
            first: Box::new(SplitNode::Leaf(leaf_id)),
            second: Box::new(SplitNode::Leaf(new_pane)),
        });
        self.active = new_pane;
        new_pane
    }

    fn replace_leaf(node: &mut SplitNode, target: PaneId, f: impl FnOnce(PaneId) -> SplitNode + Copy) {
        match node {
            SplitNode::Leaf(id) if *id == target => {
                *node = f(*id);
            }
            SplitNode::Split { first, second, .. } => {
                Self::replace_leaf(first, target, f);
                Self::replace_leaf(second, target, f);
            }
            _ => {}
        }
    }

    /// Verilen pane'i kapatır: kardeşini kendi yerine yükseltir. Son pane
    /// kapatılamaz (en az bir pane her zaman kalmalı); bu durumda `None`
    /// döner. Kapatılan pane aktifse, kardeşi (ya da onun ilk yaprağı)
    /// yeni aktif pane olur.
    pub fn close_pane(&mut self, target: PaneId) -> Option<PaneId> {
        if matches!(&self.root, SplitNode::Leaf(id) if *id == target) {
            return None; // tek pane kaldı, kapatılamaz
        }
        let promoted = Self::close_leaf(&mut self.root, target);
        if promoted.is_some() && self.active == target {
            self.active = Self::first_leaf(&self.root);
        }
        promoted
    }

    fn first_leaf(node: &SplitNode) -> PaneId {
        match node {
            SplitNode::Leaf(id) => *id,
            SplitNode::Split { first, .. } => Self::first_leaf(first),
        }
    }

    /// `node`'un alt ağacında `target` yaprağını bulup onu barındıran
    /// split'i kardeşiyle değiştirir. Bulunup değiştirilirse kardeşin ilk
    /// yaprağının id'sini döner.
    fn close_leaf(node: &mut SplitNode, target: PaneId) -> Option<PaneId> {
        let (hit_in_first, hit_in_second) = match node {
            SplitNode::Split { first, second, .. } => (
                matches!(&**first, SplitNode::Leaf(id) if *id == target),
                matches!(&**second, SplitNode::Leaf(id) if *id == target),
            ),
            SplitNode::Leaf(_) => return None,
        };

        if hit_in_first || hit_in_second {
            if let SplitNode::Split { first, second, .. } = node {
                let surviving = if hit_in_first { second.as_ref().clone() } else { first.as_ref().clone() };
                let promoted = Self::first_leaf(&surviving);
                *node = surviving;
                return Some(promoted);
            }
        }

        if let SplitNode::Split { first, second, .. } = node {
            if let Some(p) = Self::close_leaf(first, target) {
                return Some(p);
            }
            if let Some(p) = Self::close_leaf(second, target) {
                return Some(p);
            }
        }
        None
    }

    /// Bir ayracı sürüklerken `split`'in yeni oranını doğrudan belirler
    /// (fare pozisyonundan hesaplanmış mutlak oran, 0.1..0.9 arası
    /// kenetlenir).
    pub fn set_ratio(&mut self, split: SplitId, new_ratio: f32) {
        Self::set_ratio_rec(&mut self.root, split, new_ratio.clamp(0.1, 0.9));
    }

    fn set_ratio_rec(node: &mut SplitNode, split: SplitId, new_ratio: f32) {
        if let SplitNode::Split { id, ratio, first, second, .. } = node {
            if *id == split {
                *ratio = new_ratio;
                return;
            }
            Self::set_ratio_rec(first, split, new_ratio);
            Self::set_ratio_rec(second, split, new_ratio);
        }
    }

    pub fn panes(&self) -> Vec<PaneId> {
        let mut out = Vec::new();
        Self::collect_panes(&self.root, &mut out);
        out
    }

    fn collect_panes(node: &SplitNode, out: &mut Vec<PaneId>) {
        match node {
            SplitNode::Leaf(id) => out.push(*id),
            SplitNode::Split { first, second, .. } => {
                Self::collect_panes(first, out);
                Self::collect_panes(second, out);
            }
        }
    }

    /// Pane dikdörtgenlerini, ayraç hit-test dikdörtgenlerini (fare ile
    /// sürükleme için `divider_px` genişlikte) ve her split düğümünün tam
    /// kapsayıcı dikdörtgenini (oranı fare pozisyonundan hesaplamak için)
    /// döner.
    pub fn layout(&self, x: f32, y: f32, w: f32, h: f32, divider_px: f32) -> (Vec<PaneRect>, Vec<DividerRect>, Vec<SplitRect>) {
        let mut panes = Vec::new();
        let mut dividers = Vec::new();
        let mut splits = Vec::new();
        Self::layout_node(&self.root, x, y, w, h, divider_px, &mut panes, &mut dividers, &mut splits);
        (panes, dividers, splits)
    }

    fn layout_node(
        node: &SplitNode,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        divider_px: f32,
        panes: &mut Vec<PaneRect>,
        dividers: &mut Vec<DividerRect>,
        splits: &mut Vec<SplitRect>,
    ) {
        match node {
            SplitNode::Leaf(id) => panes.push(PaneRect { pane: *id, x, y, w, h }),
            SplitNode::Split { id, direction, ratio, first, second } => {
                splits.push(SplitRect { split: *id, direction: *direction, x, y, w, h });
                let half_gap = divider_px / 2.0;
                match direction {
                    SplitDirection::Horizontal => {
                        let w1 = (w * ratio - half_gap).max(1.0);
                        let w2 = (w - w1 - divider_px).max(1.0);
                        Self::layout_node(first, x, y, w1, h, divider_px, panes, dividers, splits);
                        Self::layout_node(second, x + w1 + divider_px, y, w2, h, divider_px, panes, dividers, splits);
                        dividers.push(DividerRect {
                            split: *id,
                            direction: *direction,
                            x: x + w1,
                            y,
                            w: divider_px,
                            h,
                        });
                    }
                    SplitDirection::Vertical => {
                        let h1 = (h * ratio - half_gap).max(1.0);
                        let h2 = (h - h1 - divider_px).max(1.0);
                        Self::layout_node(first, x, y, w, h1, divider_px, panes, dividers, splits);
                        Self::layout_node(second, x, y + h1 + divider_px, w, h2, divider_px, panes, dividers, splits);
                        dividers.push(DividerRect {
                            split: *id,
                            direction: *direction,
                            x,
                            y: y + h1,
                            w,
                            h: divider_px,
                        });
                    }
                }
            }
        }
    }
}

impl Clone for SplitNode {
    fn clone(&self) -> Self {
        match self {
            SplitNode::Leaf(id) => SplitNode::Leaf(*id),
            SplitNode::Split { id, direction, ratio, first, second } => SplitNode::Split {
                id: *id,
                direction: *direction,
                ratio: *ratio,
                first: first.clone(),
                second: second.clone(),
            },
        }
    }
}
