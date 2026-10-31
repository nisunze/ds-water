//! Sparse symmetric positive-definite solve: minimum-degree ordering, a
//! symbolic factorisation done once, and a numeric Cholesky repeated each
//! Newton iteration.
//!
//! The GGA matrix has the network's junction graph as its pattern, which never
//! changes while a run iterates; only the values do. Water networks are mostly
//! trees with small loops, so minimum degree eliminates leaves first and the
//! factor fills in barely at all. That is what lets one machine solve a
//! country-wide network.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

/// Symbolic structure of `L` for a fixed pattern, in the permuted order.
#[derive(Debug, Clone)]
pub struct Symbolic {
    pub n: usize,
    /// `perm[new] = old`.
    pub perm: Vec<usize>,
    /// `iperm[old] = new`.
    pub iperm: Vec<usize>,
    /// CSC of the strictly lower triangle of `L`; rows ascending per column.
    pub col_ptr: Vec<usize>,
    pub row_idx: Vec<usize>,
}

impl Symbolic {
    /// Order and analyse a symmetric pattern given as undirected edges among
    /// `n` unknowns. Duplicate and self edges are ignored.
    pub fn analyse(n: usize, edges: &[(usize, usize)]) -> Self {
        let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
        for &(a, b) in edges {
            if a != b {
                adj[a].push(b);
                adj[b].push(a);
            }
        }
        for list in &mut adj {
            list.sort_unstable();
            list.dedup();
        }

        // Minimum degree on the elimination graph. Eliminating `v` joins its
        // remaining neighbours into a clique; those neighbours are exactly the
        // rows of column `v` of `L`.
        let mut eliminated = vec![false; n];
        let mut heap: BinaryHeap<Reverse<(usize, usize)>> =
            (0..n).map(|v| Reverse((adj[v].len(), v))).collect();
        let mut perm = Vec::with_capacity(n);
        let mut columns: Vec<Vec<usize>> = Vec::with_capacity(n);
        while let Some(Reverse((degree, v))) = heap.pop() {
            if eliminated[v] || degree != adj[v].len() {
                continue;
            }
            eliminated[v] = true;
            perm.push(v);
            let neighbours = std::mem::take(&mut adj[v]);
            for &u in &neighbours {
                let list = &mut adj[u];
                if let Ok(pos) = list.binary_search(&v) {
                    list.remove(pos);
                }
                for &w in &neighbours {
                    if w != u
                        && let Err(pos) = list.binary_search(&w)
                    {
                        list.insert(pos, w);
                    }
                }
            }
            for &u in &neighbours {
                heap.push(Reverse((adj[u].len(), u)));
            }
            columns.push(neighbours);
        }

        let mut iperm = vec![0; n];
        for (new, &old) in perm.iter().enumerate() {
            iperm[old] = new;
        }
        let mut col_ptr = Vec::with_capacity(n + 1);
        let mut row_idx = Vec::new();
        col_ptr.push(0);
        for column in &columns {
            let mut rows: Vec<usize> = column.iter().map(|&old| iperm[old]).collect();
            rows.sort_unstable();
            row_idx.extend(rows);
            col_ptr.push(row_idx.len());
        }
        Self {
            n,
            perm,
            iperm,
            col_ptr,
            row_idx,
        }
    }

    /// Position in the lower-triangle value array of the entry joining two
    /// original unknowns, or `None` when they are the same unknown.
    pub fn position(&self, a: usize, b: usize) -> Option<usize> {
        let (pa, pb) = (self.iperm[a], self.iperm[b]);
        let (row, col) = match pa.cmp(&pb) {
            std::cmp::Ordering::Equal => return None,
            std::cmp::Ordering::Greater => (pa, pb),
            std::cmp::Ordering::Less => (pb, pa),
        };
        let rows = &self.row_idx[self.col_ptr[col]..self.col_ptr[col + 1]];
        rows.binary_search(&row).ok().map(|i| self.col_ptr[col] + i)
    }

    pub fn factor_nonzeros(&self) -> usize {
        self.row_idx.len()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum FactorError {
    /// The pivot of this original unknown was not positive: the matrix is not
    /// positive definite there (a junction with no path to a fixed head).
    NotPositiveDefinite { unknown: usize },
}

/// Reusable numeric workspace for one symbolic structure.
pub struct Cholesky {
    sym: Symbolic,
    l: Vec<f64>,
    diag: Vec<f64>,
    work: Vec<f64>,
    next: Vec<usize>,
    head: Vec<usize>,
    cursor: Vec<usize>,
}

const NONE: usize = usize::MAX;

impl Cholesky {
    pub fn new(sym: Symbolic) -> Self {
        let n = sym.n;
        let nnz = sym.row_idx.len();
        Self {
            sym,
            l: vec![0.0; nnz],
            diag: vec![0.0; n],
            work: vec![0.0; n],
            next: vec![NONE; n],
            head: vec![NONE; n],
            cursor: vec![0; n],
        }
    }

    pub fn symbolic(&self) -> &Symbolic {
        &self.sym
    }

    /// Factor `A = L Lᵀ`. `diag` is indexed by original unknown; `lower`
    /// holds off-diagonal values at [`Symbolic::position`] (fill entries 0).
    pub fn factor(&mut self, diag: &[f64], lower: &[f64]) -> Result<(), FactorError> {
        let n = self.sym.n;
        let (col_ptr, row_idx) = (&self.sym.col_ptr, &self.sym.row_idx);
        self.head.fill(NONE);
        // Left-looking: column j gathers updates from every earlier column k
        // with L[j,k] ≠ 0. Each k sits in the linked list of the next row it
        // must update, advancing through its column as j grows.
        for j in 0..n {
            let (start, end) = (col_ptr[j], col_ptr[j + 1]);
            for p in start..end {
                self.work[row_idx[p]] = lower[p];
            }
            let mut d = diag[self.sym.perm[j]];
            let mut k = self.head[j];
            while k != NONE {
                let next_k = self.next[k];
                let p = self.cursor[k];
                let ljk = self.l[p];
                d -= ljk * ljk;
                let k_end = col_ptr[k + 1];
                for q in p + 1..k_end {
                    self.work[row_idx[q]] -= self.l[q] * ljk;
                }
                self.cursor[k] = p + 1;
                if p + 1 < k_end {
                    let r = row_idx[p + 1];
                    self.next[k] = self.head[r];
                    self.head[r] = k;
                }
                k = next_k;
            }
            if !d.is_finite() || d <= 0.0 {
                return Err(FactorError::NotPositiveDefinite {
                    unknown: self.sym.perm[j],
                });
            }
            let pivot = d.sqrt();
            self.diag[j] = pivot;
            for p in start..end {
                let r = row_idx[p];
                self.l[p] = self.work[r] / pivot;
                self.work[r] = 0.0;
            }
            if start < end {
                self.cursor[j] = start;
                let r = row_idx[start];
                self.next[j] = self.head[r];
                self.head[r] = j;
            }
        }
        Ok(())
    }

    /// Solve `A x = b` in place; `b` is indexed by original unknown.
    pub fn solve(&mut self, b: &mut [f64]) {
        let n = self.sym.n;
        let (col_ptr, row_idx) = (&self.sym.col_ptr, &self.sym.row_idx);
        let y = &mut self.work;
        for new in 0..n {
            y[new] = b[self.sym.perm[new]];
        }
        for j in 0..n {
            y[j] /= self.diag[j];
            let yj = y[j];
            for p in col_ptr[j]..col_ptr[j + 1] {
                y[row_idx[p]] -= self.l[p] * yj;
            }
        }
        for j in (0..n).rev() {
            let mut s = y[j];
            for p in col_ptr[j]..col_ptr[j + 1] {
                s -= self.l[p] * y[row_idx[p]];
            }
            y[j] = s / self.diag[j];
        }
        for new in 0..n {
            b[self.sym.perm[new]] = y[new];
            y[new] = 0.0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Dense reference: A from a weighted Laplacian plus a grounded diagonal.
    fn system(
        n: usize,
        edges: &[(usize, usize, f64)],
        ground: &[f64],
    ) -> (Symbolic, Vec<f64>, Vec<f64>, Vec<Vec<f64>>) {
        let pattern: Vec<(usize, usize)> = edges.iter().map(|&(a, b, _)| (a, b)).collect();
        let sym = Symbolic::analyse(n, &pattern);
        let mut diag = ground.to_vec();
        let mut lower = vec![0.0; sym.factor_nonzeros()];
        let mut dense = vec![vec![0.0; n]; n];
        for i in 0..n {
            dense[i][i] = ground[i];
        }
        for &(a, b, w) in edges {
            diag[a] += w;
            diag[b] += w;
            lower[sym.position(a, b).unwrap()] -= w;
            dense[a][a] += w;
            dense[b][b] += w;
            dense[a][b] -= w;
            dense[b][a] -= w;
        }
        (sym, diag, lower, dense)
    }

    fn check(n: usize, edges: &[(usize, usize, f64)], ground: &[f64]) {
        let (sym, diag, lower, dense) = system(n, edges, ground);
        let mut chol = Cholesky::new(sym);
        chol.factor(&diag, &lower).unwrap();
        let x_true: Vec<f64> = (0..n).map(|i| (i as f64 * 0.37).sin() + 2.0).collect();
        let mut b: Vec<f64> = (0..n)
            .map(|i| (0..n).map(|j| dense[i][j] * x_true[j]).sum())
            .collect();
        chol.solve(&mut b);
        for i in 0..n {
            assert!(
                (b[i] - x_true[i]).abs() < 1e-9,
                "x[{i}] = {} vs {}",
                b[i],
                x_true[i]
            );
        }
    }

    #[test]
    fn solves_a_grounded_grid_with_fill() {
        // A 6×6 grid: loops force fill-in.
        let side = 6;
        let mut edges = Vec::new();
        for r in 0..side {
            for c in 0..side {
                let v = r * side + c;
                if c + 1 < side {
                    edges.push((v, v + 1, 1.0 + (v % 3) as f64));
                }
                if r + 1 < side {
                    edges.push((v, v + side, 0.5 + (v % 5) as f64));
                }
            }
        }
        let mut ground = vec![0.0; side * side];
        ground[0] = 3.0;
        check(side * side, &edges, &ground);
    }

    #[test]
    fn a_tree_has_no_fill() {
        let n = 200;
        let edges: Vec<(usize, usize, f64)> = (1..n)
            .map(|v| ((v - 1) / 3, v, 1.0 + v as f64 * 0.01))
            .collect();
        let mut ground = vec![0.0; n];
        ground[0] = 1.0;
        let pattern: Vec<(usize, usize)> = edges.iter().map(|&(a, b, _)| (a, b)).collect();
        assert_eq!(Symbolic::analyse(n, &pattern).factor_nonzeros(), n - 1);
        check(n, &edges, &ground);
    }

    #[test]
    fn an_ungrounded_component_refuses() {
        // Two components; only the first touches a fixed head.
        let edges = [(0, 1, 1.0), (2, 3, 1.0)];
        let (sym, diag, lower, _) = system(4, &edges, &[1.0, 0.0, 0.0, 0.0]);
        let mut chol = Cholesky::new(sym);
        assert!(matches!(
            chol.factor(&diag, &lower),
            Err(FactorError::NotPositiveDefinite { .. })
        ));
    }
}
