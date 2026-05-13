// --- Union-Find ---
#[derive(Debug, Clone)]
pub struct UnionFind {
    parent: Vec<usize>,
    rank: Vec<usize>,
}

impl UnionFind {
    pub fn new(size: usize) -> Self {
        Self {
            parent: (0..size).collect(),
            rank: vec![0; size],
        }
    }

    pub fn find(&mut self, i: usize) -> usize {
        if self.parent[i] != i {
            self.parent[i] = self.find(self.parent[i]);
        }
        self.parent[i]
    }

    pub fn union(&mut self, i: usize, j: usize) {
        let root_i = self.find(i);
        let root_j = self.find(j);

        if root_i != root_j {
            match self.rank[root_i].cmp(&self.rank[root_j]) {
                std::cmp::Ordering::Less => self.parent[root_i] = root_j,
                std::cmp::Ordering::Greater => self.parent[root_j] = root_i,
                std::cmp::Ordering::Equal => {
                    self.parent[root_j] = root_i;
                    self.rank[root_i] += 1;
                }
            }
        }
    }
}
