//! Piecewise-linear curves (gain over distance and similar).

/// A piecewise-linear function given by `(x, y)` points sorted by `x`. Outside the first and
/// last point it stays at their `y`.
#[derive(Debug, Clone, PartialEq)]
pub struct Curve {
    points: Vec<(f32, f32)>,
}

impl Curve {
    /// A curve through `points` (sorted by `x` here). No points means the constant 1.
    pub fn new(points: impl IntoIterator<Item = (f32, f32)>) -> Self {
        let mut points: Vec<(f32, f32)> = points.into_iter().collect();
        points.sort_by(|a, b| a.0.total_cmp(&b.0));
        Self { points }
    }

    /// The constant `value`.
    pub fn constant(value: f32) -> Self {
        Self::new([(0.0, value)])
    }

    /// The points, sorted by `x`.
    pub fn points(&self) -> &[(f32, f32)] {
        &self.points
    }

    /// The same curve with every `x` multiplied by `factor` (for example a curve over
    /// `0..=1` stretched over a range in metres).
    pub fn scale_x(&self, factor: f32) -> Self {
        Self::new(self.points.iter().map(|&(x, y)| (x * factor, y)))
    }

    /// The value at `x`.
    pub fn eval(&self, x: f32) -> f32 {
        let Some(&(first_x, first_y)) = self.points.first() else {
            return 1.0;
        };
        if x.is_nan() || x <= first_x {
            return first_y;
        }
        let next = self.points.partition_point(|p| p.0 <= x);
        let Some(&(x1, y1)) = self.points.get(next) else {
            return self.points[self.points.len() - 1].1;
        };
        let (x0, y0) = self.points[next - 1];
        if x1 > x0 {
            y0 + (y1 - y0) * (x - x0) / (x1 - x0)
        } else {
            y1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interpolates_linearly_and_holds_the_ends() {
        let c = Curve::new([(10.0, 0.0), (0.0, 1.0), (5.0, 0.5)]);
        assert_eq!(c.eval(-1.0), 1.0);
        assert_eq!(c.eval(2.5), 0.75);
        assert_eq!(c.eval(7.5), 0.25);
        assert_eq!(c.eval(100.0), 0.0);
        assert_eq!(Curve::new([]).eval(3.0), 1.0);
        assert_eq!(Curve::constant(0.3).eval(1e9), 0.3);
    }

    #[test]
    fn scaling_stretches_the_x_axis() {
        let c = Curve::new([(0.0, 1.0), (1.0, 0.0)]).scale_x(200.0);
        assert_eq!(c.eval(50.0), 0.75);
    }
}
