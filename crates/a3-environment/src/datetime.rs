//! Calendar date and time of day, as the engine keeps them (`date`, `setDate`, `skipTime`).

/// A local calendar date and time of day (Gregorian calendar).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DateTime {
    /// Year, e.g. 2035.
    pub year: i32,
    /// Month 1..=12.
    pub month: u32,
    /// Day of the month 1..=31.
    pub day: u32,
    /// Local time of day in hours, `0.0..24.0`.
    pub hours: f64,
}

/// `true` for Gregorian leap years.
pub fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

/// Days in `month` (1..=12) of `year`.
pub fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

impl DateTime {
    /// A date and time; `hours` is wrapped into `0..24` with the date adjusted.
    pub fn new(year: i32, month: u32, day: u32, hours: f64) -> Self {
        let mut dt = DateTime {
            year,
            month,
            day,
            hours: 0.0,
        };
        dt.skip_hours(hours);
        dt
    }

    /// Parses the world config's `startDate` (`"day/month/year"`) and `startTime`
    /// (`"hh:mm"`). `None` when malformed or not a real date.
    pub fn from_config(date: &str, time: &str) -> Option<Self> {
        let mut d = date.trim().split('/').map(|p| p.trim().parse::<i64>());
        let (day, month, year) = (d.next()?.ok()?, d.next()?.ok()?, d.next()?.ok()?);
        if d.next().is_some() {
            return None;
        }
        let mut t = time.trim().split(':').map(|p| p.trim().parse::<f64>());
        let hours = t.next()?.ok()?;
        let minutes = t.next().transpose().ok()?.unwrap_or(0.0);
        let (year, month, day) = (
            i32::try_from(year).ok()?,
            u32::try_from(month).ok()?,
            u32::try_from(day).ok()?,
        );
        if !(1..=12).contains(&month) || day < 1 || day > days_in_month(year, month) {
            return None;
        }
        if !(0.0..24.0).contains(&hours) || !(0.0..60.0).contains(&minutes) {
            return None;
        }
        Some(DateTime {
            year,
            month,
            day,
            hours: hours + minutes / 60.0,
        })
    }

    /// Day of the year, 1 for 1 January.
    pub fn day_of_year(&self) -> u32 {
        (1..self.month)
            .map(|m| days_in_month(self.year, m))
            .sum::<u32>()
            + self.day
    }

    /// Moves the time by `hours` (negative goes back), rolling the date (`skipTime`).
    pub fn skip_hours(&mut self, hours: f64) {
        let total = self.hours + hours;
        let days = (total / 24.0).floor();
        self.hours = total - days * 24.0;
        if self.hours >= 24.0 {
            self.hours -= 24.0;
        }
        let mut days = days as i64;
        while days > 0 {
            self.day += 1;
            if self.day > days_in_month(self.year, self.month) {
                self.day = 1;
                self.month += 1;
                if self.month > 12 {
                    self.month = 1;
                    self.year += 1;
                }
            }
            days -= 1;
        }
        while days < 0 {
            if self.day > 1 {
                self.day -= 1;
            } else {
                if self.month > 1 {
                    self.month -= 1;
                } else {
                    self.month = 12;
                    self.year -= 1;
                }
                self.day = days_in_month(self.year, self.month);
            }
            days += 1;
        }
    }

    /// The Julian day of this local time in a zone `utc_offset_hours` ahead of UT.
    pub fn julian_day(&self, utc_offset_hours: f64) -> f64 {
        // Fliegel and Van Flandern's day number for the Gregorian calendar (noon-based).
        let (y, m, d) = (
            i64::from(self.year),
            i64::from(self.month),
            i64::from(self.day),
        );
        let a = (14 - m) / 12;
        let y = y + 4800 - a;
        let m = m + 12 * a - 3;
        let jdn = d + (153 * m + 2) / 5 + 365 * y + y / 4 - y / 100 + y / 400 - 32045;
        jdn as f64 + (self.hours - utc_offset_hours - 12.0) / 24.0
    }
}
