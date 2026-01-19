//! Slug generation strategies.

use crate::Slug;
use crate::SlugGenerator;

use crate::base62::encode_u64;

/// Base62 encoder-based slug generator. Deterministic w.r.t. `next_id`.
/// If `min_width` is set, left-pads with '0' to reach the minimal length.
#[derive(Clone, Copy, Debug)]
pub struct Base62SlugGenerator {
    min_width: usize,
}

impl Base62SlugGenerator {
    pub fn new(min_width: usize) -> Self {
        Self { min_width }
    }
}

impl SlugGenerator for Base62SlugGenerator {
    fn next_slug(&self, next_id: u64) -> Slug {
        let mut s = encode_u64(next_id);
        if self.min_width > 0 && s.len() < self.min_width {
            let pad = self.min_width - s.len();
            let mut buf = String::with_capacity(self.min_width);
            for _ in 0..pad {
                buf.push('0');
            }
            buf.push_str(&s);
            s = buf;
        }
        // Valid by construction — base62 and '0' pad are alnum
        // If this fails (shouldn't), fall back to a safe minimal slug
        Slug::new(s).unwrap_or_else(|_| Slug::new("0").expect("'0' is valid"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_mapping() {
        let g = Base62SlugGenerator::new(0);
        assert_eq!(g.next_slug(0).as_str(), "0");
        assert_eq!(g.next_slug(61).as_str(), "z");
        assert_eq!(g.next_slug(62).as_str(), "10");
    }

    #[test]
    fn min_width_padding() {
        let g = Base62SlugGenerator::new(4);
        assert_eq!(g.next_slug(0).as_str(), "0000");
        assert_eq!(g.next_slug(1).as_str(), "0001");
        assert_eq!(g.next_slug(62).as_str(), "0010");
        assert_eq!(g.next_slug(3843).as_str(), "00zz");
        let g2 = Base62SlugGenerator::new(2);
        assert_eq!(g2.next_slug(3843).as_str(), "zz");
    }
}
