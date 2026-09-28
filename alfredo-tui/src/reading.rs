//! Durable reading positions anchored to stable transcript blocks, with legacy logical-line migration.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(
    tag = "kind",
    content = "id",
    rename_all = "kebab-case",
    deny_unknown_fields
)]
pub enum BlockKey {
    Message(usize),
    TaskReceipt(u64),
    Command(u64),
}

#[derive(Clone, Copy, Debug)]
pub struct Block {
    pub key: BlockKey,
    pub start: usize,
    pub len: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct BlockAnchor {
    key: BlockKey,
    line: usize,
    row: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Position {
    pub line: usize,
    pub row: usize,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Viewport {
    anchor: Option<Position>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    block_anchor: Option<BlockAnchor>,
    last_offset: u16,
    #[serde(skip)]
    pending_rows: i64,
}
impl Viewport {
    pub fn is_unanchored(value: &std::cell::Cell<Self>) -> bool {
        value.get().anchor.is_none()
    }
    pub fn checkpoint(self, offset: u16) -> Self {
        if self.anchor.is_none() || self.last_offset != offset {
            Self::default()
        } else {
            Self {
                pending_rows: 0,
                ..self
            }
        }
    }
    pub fn valid(&self, schema: u32, offset: u16, max_lines: usize, max_row: usize) -> bool {
        if self.block_anchor.is_some_and(|anchor| {
            schema < 9 || self.anchor.is_none() || anchor.line >= max_lines || anchor.row > max_row
        }) {
            return false;
        }
        match self.anchor {
            None => self.last_offset == 0,
            Some(anchor) => {
                schema >= 4
                    && offset > 0
                    && self.last_offset == offset
                    && anchor.line < max_lines
                    && anchor.row <= max_row
            }
        }
    }

    pub fn block_key(&self) -> Option<BlockKey> {
        self.block_anchor.map(|anchor| anchor.key)
    }
    pub fn block_anchor(&self) -> Option<(BlockKey, usize)> {
        self.block_anchor.map(|anchor| (anchor.key, anchor.line))
    }

    pub fn position_blocks(
        &mut self,
        persisted_offset: &std::cell::Cell<u16>,
        heights: &[usize],
        height: u16,
        blocks: &[Block],
    ) -> Position {
        // Explicit scroll changes take precedence over any saved anchor.
        if persisted_offset.get() != 0 && persisted_offset.get() == self.last_offset {
            if let Some(anchor) = self.block_anchor {
                if let Some(block) = blocks.iter().find(|block| block.key == anchor.key) {
                    self.anchor = Some(Position {
                        line: block
                            .start
                            .saturating_add(anchor.line.min(block.len.saturating_sub(1))),
                        row: anchor.row,
                    });
                }
            }
        }
        let position = self.position(persisted_offset, heights, height);
        self.block_anchor = self.anchor.and_then(|anchor| {
            blocks
                .iter()
                .find(|block| {
                    anchor.line >= block.start
                        && anchor.line < block.start.saturating_add(block.len)
                })
                .map(|block| BlockAnchor {
                    key: block.key,
                    line: anchor.line - block.start,
                    row: anchor.row,
                })
        });
        position
    }

    pub fn move_rows(&mut self, rows: i32) {
        self.pending_rows = self.pending_rows.saturating_add(i64::from(rows));
    }
    pub fn position(
        &mut self,
        persisted_offset: &std::cell::Cell<u16>,
        heights: &[usize],
        height: u16,
    ) -> Position {
        self.block_anchor = None;
        let total: usize = heights.iter().sum();
        let end = total.saturating_sub(usize::from(height));
        let offset = persisted_offset.get();
        let top = if offset == 0 || offset != self.last_offset {
            end.saturating_sub(usize::from(offset))
        } else if let Some(anchor) = self.anchor {
            let line = anchor.line.min(heights.len().saturating_sub(1));
            heights[..line].iter().sum::<usize>()
                + anchor
                    .row
                    .min(heights.get(line).copied().unwrap_or(1).saturating_sub(1))
        } else {
            end.saturating_sub(usize::from(offset))
        };
        let top = (top as i64)
            .saturating_add(self.pending_rows)
            .clamp(0, end as i64) as usize;
        let mut row = top;
        let mut position = Position::default();
        for (line, rows) in heights.iter().enumerate() {
            position = Position { line, row };
            if row < *rows {
                break;
            }
            row = row.saturating_sub(*rows);
        }
        self.anchor = (top < end).then_some(position);
        self.last_offset = end.saturating_sub(top).min(u16::MAX as usize) as u16;
        persisted_offset.set(self.last_offset);
        self.pending_rows = 0;
        position
    }
}
