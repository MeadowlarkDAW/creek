use std::convert::TryFrom;
use std::io::{self, Read, Write};

mod bit_depth;
mod header;

use bit_depth::BitDepth;
use header::Header;

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
}
