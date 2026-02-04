use std::collections::HashMap;

#[derive(Debug)]
pub struct OrderSeqState {
    next_seq: i64,
    by_key: HashMap<String, i64>,
}

impl OrderSeqState {
    pub fn new(next_seq: i64) -> Self {
        Self {
            next_seq: next_seq.max(1),
            by_key: HashMap::new(),
        }
    }

    pub fn get_or_assign(&mut self, key: String, existing: Option<i64>) -> i64 {
        if let Some(seq) = self.by_key.get(&key) {
            return *seq;
        }
        if let Some(seq) = existing {
            self.bump_next(seq);
            self.by_key.insert(key, seq);
            return seq;
        }
        let seq = self.next_seq;
        self.next_seq = self.next_seq.saturating_add(1);
        self.by_key.insert(key, seq);
        seq
    }

    fn bump_next(&mut self, seq: i64) {
        if seq >= self.next_seq {
            self.next_seq = seq.saturating_add(1);
        }
    }
}
