//! The `dither(a,b)` ordered dither matrix.

/// Builds the `size` x `size` dither matrix (row-major, one value per pixel).
///
/// The matrix is built level by level over 2x2, 4x4, ... blocks. At each level every block is
/// split in quadrants that receive, top-left / top-right / bottom-left / bottom-right,
/// `a`, `(a - b) * 2 / 4 + b`, `b - (b - a) / 4` and `(a - b) * 3 / 4 + b` (integer division
/// truncating toward zero). After a level `b` becomes `(b - a) / 4` and `a` becomes 0. Values
/// accumulate in wrapping 16-bit cells; the result adds `b / 2` and saturates at 255.
pub(crate) fn matrix(size: usize, mut a: i32, mut b: i32) -> Vec<u8> {
    let mut cells = vec![0u16; size * size];
    let mut add = |row: usize, col: usize, value: i32| {
        let cell = &mut cells[row * size + col];
        *cell = cell.wrapping_add(value as u16);
    };
    if size < 2 {
        b -= a;
    }
    let mut step = 2;
    while step <= size {
        let half = step / 2;
        let top_right = (a - b) * 2 / 4 + b;
        let bottom_left = b - (b - a) / 4;
        let bottom_right = (a - b) * 3 / 4 + b;
        for block_col in (0..size).step_by(step) {
            for block_row in (0..size).step_by(step) {
                for r in 0..half {
                    for c in 0..half {
                        add(block_row + r, block_col + c, a);
                        add(block_row + r, block_col + half + c, top_right);
                        add(block_row + half + r, block_col + c, bottom_left);
                        add(block_row + half + r, block_col + half + c, bottom_right);
                    }
                }
            }
        }
        step *= 2;
        b = (b - a) / 4;
        a = 0;
    }
    cells
        .iter()
        .map(|&cell| (u32::from(cell).wrapping_add((b / 2) as u32)).min(255) as u8)
        .collect()
}
