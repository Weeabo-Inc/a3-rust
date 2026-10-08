//! Dense 2D grids indexed by cell `(x, z)`.

/// The dimensions of a grid, in cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GridSize {
    /// Cells along world x (east).
    pub width: u32,
    /// Cells along world z (north).
    pub height: u32,
}

impl GridSize {
    /// A `width` x `height` grid.
    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    /// Number of cells.
    pub const fn len(self) -> usize {
        self.width as usize * self.height as usize
    }

    /// Returns `true` when the grid has no cells.
    pub const fn is_empty(self) -> bool {
        self.len() == 0
    }
}

/// A dense row-major grid: row `z` (south to north), column `x` (west to east).
///
/// Cell `(x, z)` is at index `z * width + x`, the order the engine stores its terrain arrays in.
#[derive(Clone, PartialEq)]
pub struct Grid<T> {
    size: GridSize,
    data: Vec<T>,
}

impl<T> Grid<T> {
    /// Wraps `data` (row-major, `size.len()` items) as a grid. Returns `None` on a length
    /// mismatch.
    pub fn from_vec(size: GridSize, data: Vec<T>) -> Option<Self> {
        (data.len() == size.len()).then_some(Self { size, data })
    }

    /// The grid dimensions.
    pub fn size(&self) -> GridSize {
        self.size
    }

    /// Cells along x.
    pub fn width(&self) -> u32 {
        self.size.width
    }

    /// Cells along z.
    pub fn height(&self) -> u32 {
        self.size.height
    }

    /// The cell at `(x, z)`, or `None` outside the grid.
    pub fn get(&self, x: u32, z: u32) -> Option<&T> {
        (x < self.size.width && z < self.size.height)
            .then(|| &self.data[z as usize * self.size.width as usize + x as usize])
    }

    /// Mutable access to the cell at `(x, z)`, or `None` outside the grid.
    pub fn get_mut(&mut self, x: u32, z: u32) -> Option<&mut T> {
        (x < self.size.width && z < self.size.height)
            .then(|| &mut self.data[z as usize * self.size.width as usize + x as usize])
    }

    /// The cell at `(x, z)` with both coordinates clamped into the grid.
    ///
    /// # Panics
    /// When the grid is empty.
    pub fn get_clamped(&self, x: i64, z: i64) -> &T {
        let x = x.clamp(0, i64::from(self.size.width) - 1) as usize;
        let z = z.clamp(0, i64::from(self.size.height) - 1) as usize;
        &self.data[z * self.size.width as usize + x]
    }

    /// All cells, row-major.
    pub fn as_slice(&self) -> &[T] {
        &self.data
    }

    /// All cells, row-major, mutably.
    pub fn as_mut_slice(&mut self) -> &mut [T] {
        &mut self.data
    }

    /// One row (constant `z`), west to east.
    pub fn row(&self, z: u32) -> &[T] {
        let w = self.size.width as usize;
        &self.data[z as usize * w..(z as usize + 1) * w]
    }

    /// Unwraps the row-major cell vector.
    pub fn into_vec(self) -> Vec<T> {
        self.data
    }
}

impl<T: Clone> Grid<T> {
    /// A grid with every cell set to `value`.
    pub fn filled(size: GridSize, value: T) -> Self {
        Self {
            size,
            data: vec![value; size.len()],
        }
    }
}

impl<T: Default + Clone> Grid<T> {
    /// A grid of default values.
    pub fn new(size: GridSize) -> Self {
        Self::filled(size, T::default())
    }
}

impl<T> std::fmt::Debug for Grid<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Grid({}x{})", self.size.width, self.size.height)
    }
}
