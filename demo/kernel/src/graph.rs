//! A directed graph: walks, orders and connected pieces.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};

use crate::error::{KernelError, KernelResult};

/// A directed graph kept as sorted adjacency lists.
///
/// The map is sorted, so a walk visits neighbours in a fixed order and two
/// graphs built from the same edges answer identically. The structure is small
/// on purpose: the interesting part is the traversal, not the storage.
#[derive(Debug, Clone, Default)]
pub struct Graph {
    edges: BTreeMap<u64, Vec<u64>>,
}

impl Graph {
    /// A graph with no edges.
    pub fn new() -> Self {
        Self::default()
    }

    /// A graph built from `(from, to)` pairs.
    ///
    /// A node that only appears as a target is added with no outgoing edges, so
    /// it still counts as a node.
    pub fn from_edges(edges: &[(u64, u64)]) -> Self {
        let mut graph = Self::new();
        for (from, to) in edges {
            graph.edges.entry(*from).or_default().push(*to);
            graph.edges.entry(*to).or_default();
        }
        for neighbours in graph.edges.values_mut() {
            neighbours.sort_unstable();
            neighbours.dedup();
        }
        graph
    }

    /// The number of distinct nodes.
    pub fn nodes(&self) -> usize {
        self.edges.len()
    }

    /// The number of distinct edges.
    pub fn edge_count(&self) -> usize {
        self.edges.values().map(Vec::len).sum()
    }

    /// Reports whether no edge exists.
    pub fn is_empty(&self) -> bool {
        self.edges.is_empty()
    }

    /// The nodes `node` points at, in ascending order.
    pub fn successors(&self, node: u64) -> &[u64] {
        self.edges.get(&node).map_or(&[], Vec::as_slice)
    }

    /// Every node reachable from `start`, including it, in visit order.
    ///
    /// A `start` that no edge names is a node of its own, so the answer is just
    /// that node.
    pub fn reachable(&self, start: u64) -> Vec<u64> {
        let mut seen = BTreeSet::new();
        let mut queue = VecDeque::new();
        let mut order = Vec::new();
        queue.push_back(start);
        seen.insert(start);
        while let Some(node) = queue.pop_front() {
            order.push(node);
            for next in self.successors(node) {
                if seen.insert(*next) {
                    queue.push_back(*next);
                }
            }
        }
        order
    }

    /// The length in edges of the longest path from `start`.
    ///
    /// An edge into a node already on the current path is not followed, so a
    /// cycle answers with a finite number and the answer counts only simple
    /// paths.
    pub fn depth(&self, start: u64) -> usize {
        fn walk(graph: &Graph, node: u64, path: &mut Vec<u64>) -> usize {
            if path.contains(&node) {
                return 0;
            }
            path.push(node);
            let mut deepest = 0;
            for next in graph.successors(node) {
                if path.contains(next) {
                    continue;
                }
                deepest = deepest.max(1 + walk(graph, *next, path));
            }
            path.pop();
            deepest
        }
        walk(self, start, &mut Vec::new())
    }

    /// An order in which every edge points forward.
    ///
    /// # Errors
    ///
    /// [`KernelError::GraphCycle`] when no such order exists.
    pub fn topological(&self) -> KernelResult<Vec<u64>> {
        let mut indegree: BTreeMap<u64, usize> =
            self.edges.keys().map(|node| (*node, 0)).collect();
        for neighbours in self.edges.values() {
            for next in neighbours {
                *indegree.entry(*next).or_insert(0) += 1;
            }
        }
        let mut ready: BTreeSet<u64> = indegree
            .iter()
            .filter(|(_, count)| **count == 0)
            .map(|(node, _)| *node)
            .collect();
        let mut order = Vec::with_capacity(self.edges.len());
        while let Some(node) = ready.pop_first() {
            order.push(node);
            for next in self.successors(node) {
                if let Some(count) = indegree.get_mut(next) {
                    *count -= 1;
                    if *count == 0 {
                        ready.insert(*next);
                    }
                }
            }
        }
        if order.len() != self.edges.len() {
            return Err(KernelError::GraphCycle {
                nodes: self.edges.len() - order.len(),
            });
        }
        Ok(order)
    }

    /// The number of weakly connected pieces.
    ///
    /// Two nodes belong to one piece when an edge joins them, in either
    /// direction. The pieces are found with a union-find that compresses the
    /// path it walks.
    pub fn components(&self) -> usize {
        fn root(parent: &mut HashMap<u64, u64>, node: u64) -> u64 {
            let mut current = node;
            while parent[&current] != current {
                current = parent[&current];
            }
            let mut cursor = node;
            while parent[&cursor] != current {
                let next = parent[&cursor];
                parent.insert(cursor, current);
                cursor = next;
            }
            current
        }

        let mut parent: HashMap<u64, u64> =
            self.edges.keys().map(|node| (*node, *node)).collect();
        for (from, neighbours) in &self.edges {
            for to in neighbours {
                let left = root(&mut parent, *from);
                let right = root(&mut parent, *to);
                if left != right {
                    parent.insert(left, right);
                }
            }
        }
        let nodes: Vec<u64> = parent.keys().copied().collect();
        let mut roots = BTreeSet::new();
        for node in nodes {
            let top = root(&mut parent, node);
            roots.insert(top);
        }
        roots.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Graph {
        // 1 -> 2 -> 3, 1 -> 4
        Graph::from_edges(&[(1, 2), (2, 3), (1, 4)])
    }

    #[test]
    fn a_walk_visits_each_node_once() {
        let graph = sample();
        assert_eq!(graph.nodes(), 4);
        assert_eq!(graph.edge_count(), 3);
        assert_eq!(graph.reachable(1), vec![1, 2, 4, 3]);
        assert_eq!(graph.reachable(9), vec![9]);
    }

    #[test]
    fn the_longest_path_is_measured_in_edges() {
        let graph = sample();
        assert_eq!(graph.depth(1), 2);
        assert_eq!(graph.depth(3), 0);
        // A cycle answers with a finite number, counting only simple paths.
        let cyclic = Graph::from_edges(&[(1, 2), (2, 1)]);
        assert_eq!(cyclic.depth(1), 1);
    }

    #[test]
    fn an_order_puts_every_edge_forward() {
        let graph = sample();
        let order = graph.topological().unwrap();
        // The ready set is sorted, so the answer is the smallest node that is
        // ready at each step: 1, then 2, then 3, then 4.
        assert_eq!(order, vec![1, 2, 3, 4]);
        let cyclic = Graph::from_edges(&[(1, 2), (2, 3), (3, 1)]);
        assert!(matches!(
            cyclic.topological(),
            Err(KernelError::GraphCycle { .. })
        ));
    }

    #[test]
    fn pieces_are_joined_in_either_direction() {
        let graph = Graph::from_edges(&[(1, 2), (2, 3), (10, 11)]);
        assert_eq!(graph.components(), 2);
        assert_eq!(Graph::new().components(), 0);
        assert_eq!(Graph::from_edges(&[]).nodes(), 0);
    }
}
