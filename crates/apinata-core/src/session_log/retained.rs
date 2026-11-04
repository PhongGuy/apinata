//! Two per-kind retained queues under the session log's limits.

use super::{ConsoleLine, LogLimits, NetworkEntry};
use std::collections::VecDeque;
use std::sync::Arc;

/// One retained item of either kind.
#[derive(Clone)]
pub(crate) enum Logged {
    Console(Arc<ConsoleLine>),
    Network(Arc<NetworkEntry>),
}

impl Logged {
    pub(crate) fn accounted_bytes(&self) -> usize {
        match self {
            Logged::Console(l) => l.accounted_bytes(),
            Logged::Network(e) => e.accounted_bytes(),
        }
    }
}

/// A queue entry: the caller's order, the item and its accounted size taken at push time.
type Slot<T> = (u64, Arc<T>, usize);

/// Two per-kind queues under `LogLimits`, ordered by a caller key that only increases.
pub(crate) struct Retained {
    console: VecDeque<Slot<ConsoleLine>>,
    network: VecDeque<Slot<NetworkEntry>>,
    bytes: usize,
    limits: LogLimits,
}

impl Retained {
    pub(crate) fn new(limits: LogLimits) -> Retained {
        Retained {
            console: VecDeque::new(),
            network: VecDeque::new(),
            bytes: 0,
            limits,
        }
    }

    /// Rule 1: evict the oldest of the item's kind past its count cap. Rule 2: while over
    /// the budget, evict the lowest order of either kind; the new item is never evicted.
    pub(crate) fn push(&mut self, order: u64, item: Logged) {
        debug_assert!(
            [
                self.console.back().map(|s| s.0),
                self.network.back().map(|s| s.0)
            ]
            .into_iter()
            .flatten()
            .all(|last| last < order),
            "orders must strictly increase"
        );
        let size = item.accounted_bytes();
        self.bytes += size;
        match item {
            Logged::Console(l) => {
                self.console.push_back((order, l, size));
                while self.console.len() > self.limits.console.max(1) {
                    self.pop_console();
                }
            }
            Logged::Network(e) => {
                self.network.push_back((order, e, size));
                while self.network.len() > self.limits.network.max(1) {
                    self.pop_network();
                }
            }
        }
        // The new item holds the highest order, so it is the last one left standing:
        // popping the lower front of the two queues never reaches it while another remains.
        while self.bytes > self.limits.budget_bytes && self.console.len() + self.network.len() > 1 {
            let console_front = self.console.front().map(|s| s.0);
            let network_front = self.network.front().map(|s| s.0);
            match (console_front, network_front) {
                (Some(c), Some(n)) if c < n => self.pop_console(),
                (Some(_), None) => self.pop_console(),
                _ => self.pop_network(),
            }
        }
    }

    fn pop_console(&mut self) {
        if let Some((_, _, size)) = self.console.pop_front() {
            self.bytes -= size;
        }
    }

    fn pop_network(&mut self) {
        if let Some((_, _, size)) = self.network.pop_front() {
            self.bytes -= size;
        }
    }

    pub(crate) fn bytes(&self) -> usize {
        self.bytes
    }

    /// Console lines whose order is at least `order`, oldest first.
    pub(crate) fn console_from(&self, order: u64) -> Vec<Arc<ConsoleLine>> {
        from(&self.console, order)
    }

    /// Network entries whose order is at least `order`, oldest first.
    pub(crate) fn network_from(&self, order: u64) -> Vec<Arc<NetworkEntry>> {
        from(&self.network, order)
    }

    pub(crate) fn first_console(&self) -> Option<u64> {
        self.console.front().map(|s| s.0)
    }

    pub(crate) fn first_network(&self) -> Option<u64> {
        self.network.front().map(|s| s.0)
    }

    pub(crate) fn clear_console(&mut self) {
        while !self.console.is_empty() {
            self.pop_console();
        }
    }

    pub(crate) fn clear_network(&mut self) {
        while !self.network.is_empty() {
            self.pop_network();
        }
    }

    /// Both kinds merged by order.
    pub(crate) fn into_ordered(self) -> Vec<Logged> {
        let mut console = self.console.into_iter().peekable();
        let mut network = self.network.into_iter().peekable();
        let mut out = Vec::with_capacity(console.len() + network.len());
        loop {
            let take_console = match (console.peek(), network.peek()) {
                (Some(c), Some(n)) => c.0 < n.0,
                (Some(_), None) => true,
                (None, Some(_)) => false,
                (None, None) => break,
            };
            if take_console {
                out.extend(console.next().map(|s| Logged::Console(s.1)));
            } else {
                out.extend(network.next().map(|s| Logged::Network(s.1)));
            }
        }
        out
    }
}

/// The suffix of a queue at or after `order`; orders only increase, so it is a contiguous tail.
fn from<T>(queue: &VecDeque<Slot<T>>, order: u64) -> Vec<Arc<T>> {
    let start = queue.partition_point(|s| s.0 < order);
    queue.range(start..).map(|s| Arc::clone(&s.1)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_log::redact::Redactor;
    use crate::session_log::test_support::{entry_with, line_with};

    fn limits(console: usize, network: usize, budget_bytes: usize) -> LogLimits {
        LogLimits {
            console,
            network,
            body_bytes: 64 * 1024,
            budget_bytes,
        }
    }
    fn line(text: &str) -> Logged {
        Logged::Console(Arc::new(line_with(&Redactor::none(), text)))
    }
    fn entry(body: usize) -> Logged {
        Logged::Network(Arc::new(entry_with(
            &Redactor::none(),
            "v",
            &vec![b'b'; body],
            64 * 1024,
        )))
    }
    fn orders(r: &Retained) -> Vec<u64> {
        let mut all: Vec<u64> = r.console_from(0).iter().map(|l| l.seq).collect();
        all.extend(r.network_from(0).iter().map(|e| e.seq));
        all.sort();
        all
    }
    /// Items carry their order in `seq` so tests can read it back.
    fn with_seq(item: Logged, order: u64) -> Logged {
        match item {
            Logged::Console(l) => Logged::Console(Arc::new(ConsoleLine {
                seq: order,
                ..(*l).clone()
            })),
            Logged::Network(e) => Logged::Network(Arc::new(NetworkEntry {
                seq: order,
                ..(*e).clone()
            })),
        }
    }

    #[test]
    fn count_caps_evict_the_oldest_of_that_kind_only() {
        let mut r = Retained::new(limits(3, 2, usize::MAX));
        r.push(0, with_seq(entry(10), 0));
        for order in 1..6 {
            r.push(order, with_seq(line("l"), order));
        }
        assert_eq!(
            r.console_from(0).iter().map(|l| l.seq).collect::<Vec<_>>(),
            [3, 4, 5]
        );
        assert_eq!(r.first_console(), Some(3));
        assert_eq!(r.first_network(), Some(0));
        assert_eq!(r.console_from(4).len(), 2);
    }

    #[test]
    fn budget_evicts_lowest_orders_first_and_bytes_match_a_recount() {
        let budget = 1024 * 1024;
        let mut r = Retained::new(limits(1000, 500, budget));
        let mut pushed = 0usize;
        let mut order = 0u64;
        while pushed < 3 * budget {
            let item = if order % 3 == 2 {
                line(&"c".repeat(4000))
            } else {
                entry(64 * 1024)
            };
            let item = with_seq(item, order);
            pushed += item.accounted_bytes();
            r.push(order, item);
            assert!(r.bytes() <= budget, "order {order}: {}", r.bytes());
            let recount: usize = r
                .console_from(0)
                .iter()
                .map(|l| l.accounted_bytes())
                .sum::<usize>()
                + r.network_from(0)
                    .iter()
                    .map(|e| e.accounted_bytes())
                    .sum::<usize>();
            assert_eq!(r.bytes(), recount);
            let kept = orders(&r);
            assert_eq!(
                kept,
                (kept[0]..=order).collect::<Vec<_>>(),
                "a contiguous newest suffix"
            );
            order += 1;
        }
        assert!(orders(&r)[0] > 0);
    }

    #[test]
    fn a_new_item_is_kept_even_alone_over_the_budget() {
        let mut r = Retained::new(limits(10, 10, 16));
        r.push(0, with_seq(line("first"), 0));
        r.push(1, with_seq(line("second"), 1));
        assert_eq!(orders(&r), [1]);
        assert!(r.bytes() > 16);
    }

    #[test]
    fn clear_drops_one_kind_and_its_bytes() {
        let mut r = Retained::new(limits(10, 10, usize::MAX));
        r.push(0, with_seq(line("a"), 0));
        r.push(1, with_seq(entry(10), 1));
        let network_bytes = r.network_from(0)[0].accounted_bytes();
        r.clear_console();
        assert_eq!((r.first_console(), r.bytes()), (None, network_bytes));
        r.clear_network();
        assert_eq!((r.first_network(), r.bytes()), (None, 0));
    }

    #[test]
    fn into_ordered_interleaves_both_kinds_by_order() {
        let mut r = Retained::new(limits(10, 10, usize::MAX));
        r.push(0, with_seq(line("a"), 0));
        r.push(1, with_seq(entry(1), 1));
        r.push(2, with_seq(line("b"), 2));
        let kinds: Vec<&str> = r
            .into_ordered()
            .iter()
            .map(|i| match i {
                Logged::Console(_) => "c",
                Logged::Network(_) => "n",
            })
            .collect();
        assert_eq!(kinds, ["c", "n", "c"]);
    }

    #[test]
    #[should_panic(expected = "orders must strictly increase")]
    fn a_repeated_order_trips_the_debug_check() {
        let mut r = Retained::new(limits(10, 10, usize::MAX));
        r.push(1, with_seq(line("a"), 1));
        r.push(1, with_seq(entry(1), 1));
    }
}
