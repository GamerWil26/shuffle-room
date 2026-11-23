use rand::{seq::SliceRandom, Rng};
use std::path::PathBuf;

#[derive(Default)]
pub struct ShuffleBag {
    pub pending: Vec<PathBuf>,
    pub last: Option<PathBuf>,
    pub pass: u64,
}

impl ShuffleBag {
    pub fn refill(&mut self, mut files: Vec<PathBuf>, rng: &mut impl Rng) {
        files.shuffle(rng);
        if files.len() > 1 && files.last() == self.last.as_ref() {
            let last = files.len() - 1;
            let other = rng.gen_range(0..last);
            files.swap(last, other);
        }
        self.pending = files;
        self.pass += 1;
    }
    pub fn next(&mut self) -> Option<PathBuf> {
        self.pending.pop()
    }
    pub fn played(&mut self, path: PathBuf) {
        self.last = Some(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::{rngs::StdRng, SeedableRng};
    use std::collections::HashSet;
    #[test]
    fn permutation_and_boundary_for_many_passes() {
        let mut bag = ShuffleBag::default();
        let mut rng = StdRng::seed_from_u64(33);
        let files: Vec<_> = (0..7).map(|n| PathBuf::from(n.to_string())).collect();
        let mut orders = HashSet::new();
        for _ in 0..100 {
            let previous = bag.last.clone();
            bag.refill(files.clone(), &mut rng);
            let mut pass = Vec::new();
            while let Some(p) = bag.next() {
                bag.played(p.clone());
                pass.push(p);
            }
            assert_ne!(pass.first(), previous.as_ref());
            assert_eq!(pass.iter().collect::<HashSet<_>>().len(), 7);
            orders.insert(pass);
        }
        assert!(orders.len() > 90);
    }
    #[test]
    fn empty_and_single() {
        let mut bag = ShuffleBag::default();
        let mut rng = rand::thread_rng();
        bag.refill(vec![], &mut rng);
        assert!(bag.next().is_none());
        for _ in 0..3 {
            bag.refill(vec!["only".into()], &mut rng);
            assert_eq!(bag.next(), Some("only".into()));
            bag.played("only".into());
        }
    }
}
