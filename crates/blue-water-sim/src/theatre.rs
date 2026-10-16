pub mod content {
    /// ISO local date and minute used by the preserved sky scene's sidereal clock.
    pub fn parse_start(value: &str) -> Option<(i32, u32, u32, u32, u32)> {
        let (date, time) = value.split_once('T')?;
        let mut date = date.split('-');
        let year = date.next()?.parse::<i32>().ok()?;
        let month = date.next()?.parse::<u32>().ok()?;
        let day = date.next()?.parse::<u32>().ok()?;
        if date.next().is_some() || !(1..=12).contains(&month) {
            return None;
        }
        let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
        let days = [
            31,
            if leap { 29 } else { 28 },
            31,
            30,
            31,
            30,
            31,
            31,
            30,
            31,
            30,
            31,
        ];
        if day == 0 || day > days[(month - 1) as usize] {
            return None;
        }
        let (hour, minute) = time.split_once(':')?;
        let hour = hour.parse::<u32>().ok()?;
        let minute = minute.parse::<u32>().ok()?;
        (hour < 24 && minute < 60).then_some((year, month, day, hour, minute))
    }
}
#[path = "../../../vendor/blue-water-sea-and-sky/shared/sim/sun.rs"]
pub mod sun;
