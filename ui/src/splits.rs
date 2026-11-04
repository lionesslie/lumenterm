//! Basit bir ikili bölme (binary split) ağacı. Her yaprak bir `PaneId`
//! (bir `lumenterm_core::Session` ile eşleşir), her iç düğüm yatay/dikey
//! bir bölme ve sürüklenebilir bir oran (`ratio: 0.0..1.0`) tutar.

pub type PaneId = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitDirection {
    Horizontal,
    Vertical,
}

#[derive(Debug, Clone)]
pub enum SplitNode {
    Leaf(PaneId),
    Split {
        direction: SplitDirection,
        ratio: f32,
        first: Box<SplitNode>,
        second: Box<SplitNode>,
    },
}

pub struct SplitTree {
    pub root: SplitNode,
    pub active: PaneId,
    next_id: PaneId,
}

/// Bir pane'in ekrandaki normalize dikdörtgeni (0..1 aralığında), `app`
/// katmanı bunu pencere piksel boyutuna çarparak gerçek pane sınırlarını
/// bulur.
#[derive(Debug, Clone, Copy)]
pub struct PaneRect {
    pub pane: PaneId,
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
            next_id: 1,
        }
    }

    /// Aktif pane'i verilen yönde böler, yeni bir pane oluşturur ve onu
    /// aktif yapar. Yeni pane'in `Session`'ını kurmak `app` katmanının
    /// sorumluluğundadır (bu fonksiyon yalnızca layout'u günceller).
    pub fn split_active(&mut self, direction: SplitDirection) -> PaneId {
        let new_pane = self.next_id;
        self.next_id += 1;
        let active = self.active;
        Self::replace_leaf(&mut self.root, active, |leaf_id| SplitNode::Split {
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

    /// Ayracı sürüklerken çağrılır; basitlik için şimdilik tüm ağaçtaki
    /// ilk eşleşen split'in oranını günceller (gerçek implementasyonda
    /// `app` fare pozisyonundan hangi ayracın sürüklendiğini bulup bir
    /// path/id iletir).
    pub fn resize_split(&mut self, delta: f32) {
        Self::adjust_first_split(&mut self.root, delta);
    }

    fn adjust_first_split(node: &mut SplitNode, delta: f32) -> bool {
        match node {
            SplitNode::Split { ratio, first, second, .. } => {
                if Self::adjust_first_split(first, delta) {
                    return true;
                }
                if Self::adjust_first_split(second, delta) {
                    return true;
                }
                *ratio = (*ratio + delta).clamp(0.1, 0.9);
                true
            }
            SplitNode::Leaf(_) => false,
        }
    }

    pub fn layout(&self, x: f32, y: f32, w: f32, h: f32) -> Vec<PaneRect> {
        let mut out = Vec::new();
        Self::layout_node(&self.root, x, y, w, h, &mut out);
        out
    }

    fn layout_node(node: &SplitNode, x: f32, y: f32, w: f32, h: f32, out: &mut Vec<PaneRect>) {
        match node {
            SplitNode::Leaf(id) => out.push(PaneRect { pane: *id, x, y, w, h }),
            SplitNode::Split { direction, ratio, first, second } => match direction {
                SplitDirection::Horizontal => {
                    let w1 = w * ratio;
                    Self::layout_node(first, x, y, w1, h, out);
                    Self::layout_node(second, x + w1, y, w - w1, h, out);
                }
                SplitDirection::Vertical => {
                    let h1 = h * ratio;
                    Self::layout_node(first, x, y, w, h1, out);
                    Self::layout_node(second, x, y + h1, w, h - h1, out);
                }
            },
        }
    }
}
