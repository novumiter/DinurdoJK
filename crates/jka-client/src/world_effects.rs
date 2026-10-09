//! Jedi Academy renderer world-effect compatibility (`r_we`).
//!
//! TaystJK/OpenJK route both the local renderer command and CS_EFFECTS strings
//! beginning with `*` through the same parser.  Keep that protocol/command
//! state separate from DinurdoJK's modern weather renderer: rain/acid rain and
//! snow use dedicated GPU weather paths, sand/spacedust use a lightweight analytic
//! GPU atmosphere path, and the remaining retail effects stay typed for later.

use crate::{ocean::AtmosphericWind, ui::RainIntensity};

pub const MAX_WIND_ZONES: usize = 10;
pub const MAX_WEATHER_ZONES: usize = 10;
pub const MAX_PARTICLE_CLOUDS: usize = 5;

pub const HELP: &str = "die clear freeze zone wind constantwind gustingwind lightrain rain acidrain heavyrain snow spacedust sand fog heavyrainfog light_fog outsideshake outsidepain";

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WindZone {
    /// Retail random wind. Stored exactly as a distinct legacy zone; the modern
    /// renderer does not synthesize its random walk yet.
    Random,
    Constant([f32; 3]),
    /// Retail gusting wind. Stored as a placeholder until its legacy random
    /// target/decay behavior is promoted into the shared weather wind field.
    Gusting,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WeatherZone {
    pub mins: [f32; 3],
    pub maxs: [f32; 3],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParticleCloud {
    LightRain,
    Rain,
    AcidRain,
    HeavyRain,
    Snow,
    SpaceDust(i32),
    Sand,
    Fog,
    HeavyRainFog,
    LightFog,
}

impl ParticleCloud {
    pub fn is_rain(self) -> bool {
        matches!(self, Self::LightRain | Self::Rain | Self::AcidRain | Self::HeavyRain)
    }

    fn legacy_rain_particles(self) -> u32 {
        match self {
            Self::LightRain => 500,
            Self::Rain | Self::AcidRain | Self::HeavyRain => 1000,
            _ => 0,
        }
    }
}

/// Singular user-facing weather choice. The retail `r_we` parser remains additive
/// for map/server compatibility; the Weather settings page uses this enum to replace
/// only particle-weather clouds while keeping wind zones and other world-effect state.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum WeatherType {
    #[default]
    Clear,
    LightRain,
    Rain,
    AcidRain,
    HeavyRain,
    Snow,
    SpaceDust,
    Sand,
    Fog,
    HeavyRainFog,
    LightFog,
}

impl WeatherType {
    pub const ALL: [Self; 11] = [
        Self::Clear,
        Self::LightRain,
        Self::Rain,
        Self::AcidRain,
        Self::HeavyRain,
        Self::Snow,
        Self::SpaceDust,
        Self::Sand,
        Self::Fog,
        Self::HeavyRainFog,
        Self::LightFog,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Clear => "Clear",
            Self::LightRain => "Light rain",
            Self::Rain => "Rain",
            Self::AcidRain => "Acid rain",
            Self::HeavyRain => "Heavy rain",
            Self::Snow => "Snow",
            Self::SpaceDust => "Space dust",
            Self::Sand => "Sand",
            Self::Fog => "Fog (legacy)",
            Self::HeavyRainFog => "Heavy rain + fog (legacy)",
            Self::LightFog => "Light fog (legacy)",
        }
    }

    pub fn command(self) -> Option<&'static str> {
        match self {
            Self::Clear => None,
            Self::LightRain => Some("lightrain"),
            Self::Rain => Some("rain"),
            Self::AcidRain => Some("acidrain"),
            Self::HeavyRain => Some("heavyrain"),
            Self::Snow => Some("snow"),
            // Retail defaults a missing count to zero. Give the menu a useful
            // visible default while direct `r_we spacedust <count>` stays exact.
            Self::SpaceDust => Some("spacedust 1000"),
            Self::Sand => Some("sand"),
            Self::Fog => Some("fog"),
            Self::HeavyRainFog => Some("heavyrainfog"),
            Self::LightFog => Some("light_fog"),
        }
    }

    pub fn cvar_value(self) -> &'static str {
        match self {
            Self::Clear => "clear",
            Self::LightRain => "lightrain",
            Self::Rain => "rain",
            Self::AcidRain => "acidrain",
            Self::HeavyRain => "heavyrain",
            Self::Snow => "snow",
            Self::SpaceDust => "spacedust",
            Self::Sand => "sand",
            Self::Fog => "fog",
            Self::HeavyRainFog => "heavyrainfog",
            Self::LightFog => "light_fog",
        }
    }

    pub fn from_cvar(value: &str) -> Option<Self> {
        let value = value.trim();
        Self::ALL.into_iter().find(|weather| weather.cvar_value().eq_ignore_ascii_case(value))
    }

    pub fn is_rain(self) -> bool {
        matches!(self, Self::LightRain | Self::Rain | Self::AcidRain | Self::HeavyRain)
    }

    pub fn uses_footprints(self) -> bool {
        matches!(self, Self::Snow | Self::Sand)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RainOverride {
    pub intensity: RainIntensity,
    /// Density relative to one instance of the selected legacy rain cloud.
    /// The renderer caps the final GPU count at its existing hard maximum.
    pub density_scale: f32,
    pub tint: [f32; 3],
    pub frozen: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SnowOverride {
    /// Density relative to one stock JKA snow cloud (1,000 particles).
    pub density_scale: f32,
    /// `r_we freeze`: keep rendering the current flakes without advancing them.
    pub frozen: bool,
}

/// Modern presentation inputs distilled from the legacy non-rain particle clouds.
/// Counts remain independent from the renderer so `r_we` parsing stays faithful.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AtmosphereOverride {
    pub sand_layers: u32,
    pub space_dust_count: u32,
    pub frozen: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ApplyResult {
    Applied,
    Placeholder(&'static str),
    Capacity(&'static str),
}

#[derive(Debug, Clone, PartialEq)]
pub struct LegacyWorldEffects {
    particle_clouds: Vec<ParticleCloud>,
    wind_zones: Vec<WindZone>,
    weather_zones: Vec<WeatherZone>,
    frozen: bool,
    outside_shake: bool,
    /// Retail code stores this as float. `acidrain` assigns 0.1; `outsidepain`
    /// toggles it via C/C++ logical negation, producing exactly 0.0 or 1.0.
    outside_pain: f32,
    /// Becomes true once a particle-weather `r_we` command (including clear/die)
    /// has explicitly authored weather. Until then, old configs may still use
    /// archived `r_rain` as a startup fallback.
    weather_authored: bool,
}

impl Default for LegacyWorldEffects {
    fn default() -> Self {
        Self {
            particle_clouds: Vec::new(),
            wind_zones: Vec::new(),
            weather_zones: Vec::new(),
            frozen: false,
            outside_shake: false,
            outside_pain: 0.0,
            weather_authored: false,
        }
    }
}

impl LegacyWorldEffects {
    pub fn apply(&mut self, raw: &str) -> Result<ApplyResult, String> {
        let raw = raw.trim();
        let raw = raw.strip_prefix('*').unwrap_or(raw).trim();
        let mut words = raw.split_whitespace();
        let Some(command) = words.next() else {
            return Err(format!("r_we: {HELP}"));
        };
        let command = command.to_ascii_lowercase();

        match command.as_str() {
            "die" => {
                // R_ShutdownWorldEffects -> R_InitWorldEffects clears clouds,
                // wind, and the outside cache/zones. The renderer-global
                // mFrozen flag is intentionally not reset by that function.
                self.particle_clouds.clear();
                self.wind_zones.clear();
                self.weather_zones.clear();
                self.outside_shake = false;
                self.outside_pain = 0.0;
                self.weather_authored = true;
                Ok(ApplyResult::Applied)
            }
            "clear" => {
                // RE_WorldEffectCommand("clear") removes particle clouds and
                // wind zones only. Freeze/outside/weather-zone state survives.
                self.particle_clouds.clear();
                self.wind_zones.clear();
                self.weather_authored = true;
                Ok(ApplyResult::Applied)
            }
            "freeze" => {
                self.frozen = !self.frozen;
                Ok(ApplyResult::Applied)
            }
            "zone" => {
                if self.weather_zones.len() >= MAX_WEATHER_ZONES {
                    return Ok(ApplyResult::Capacity("weather zones"));
                }
                let mut tokens = raw.split_whitespace();
                let _ = tokens.next();
                let mins = parse_parenthesized_vec3(&mut tokens)
                    .ok_or_else(|| "r_we zone: expected ( minx miny minz ) ( maxx maxy maxz )".to_owned())?;
                let maxs = parse_parenthesized_vec3(&mut tokens)
                    .ok_or_else(|| "r_we zone: expected ( minx miny minz ) ( maxx maxy maxz )".to_owned())?;
                self.weather_zones.push(WeatherZone { mins, maxs });
                Ok(ApplyResult::Placeholder("weather zone"))
            }
            "wind" => self.push_wind(WindZone::Random, "random wind"),
            "constantwind" => {
                // The retail parser falls back to (0,800,0) when sscanf fails.
                let mut tokens = raw.split_whitespace();
                let _ = tokens.next();
                let vector = parse_parenthesized_vec3(&mut tokens).unwrap_or([0.0, 800.0, 0.0]);
                self.push_wind(WindZone::Constant(vector), "constant wind")
            }
            "gustingwind" => self.push_wind(WindZone::Gusting, "gusting wind"),
            "lightrain" => self.push_cloud(ParticleCloud::LightRain, None),
            "rain" => self.push_cloud(ParticleCloud::Rain, None),
            "acidrain" => {
                if self.particle_clouds.len() >= MAX_PARTICLE_CLOUDS {
                    return Ok(ApplyResult::Capacity("particle clouds"));
                }
                self.particle_clouds.push(ParticleCloud::AcidRain);
                self.weather_authored = true;
                // Retail sets this only after successfully allocating the cloud.
                self.outside_pain = 0.1;
                Ok(ApplyResult::Applied)
            }
            "heavyrain" => self.push_cloud(ParticleCloud::HeavyRain, None),
            "snow" => self.push_cloud(ParticleCloud::Snow, None),
            "spacedust" => {
                // atoi("") == 0 in the C implementation.
                let count = words.next().and_then(|v| v.parse::<i32>().ok()).unwrap_or(0);
                self.push_cloud(ParticleCloud::SpaceDust(count), None)
            }
            "sand" => self.push_cloud(ParticleCloud::Sand, None),
            "fog" => self.push_cloud(ParticleCloud::Fog, Some("fog")),
            "heavyrainfog" => self.push_cloud(ParticleCloud::HeavyRainFog, Some("heavy-rain fog")),
            "light_fog" => self.push_cloud(ParticleCloud::LightFog, Some("light fog")),
            "outsideshake" => {
                self.outside_shake = !self.outside_shake;
                Ok(ApplyResult::Placeholder("outside shake"))
            }
            "outsidepain" => {
                self.outside_pain = if self.outside_pain == 0.0 { 1.0 } else { 0.0 };
                Ok(ApplyResult::Placeholder("outside pain"))
            }
            _ => Err(format!("r_we: unknown world effect '{command}'\nr_we: {HELP}")),
        }
    }

    fn push_wind(&mut self, zone: WindZone, label: &'static str) -> Result<ApplyResult, String> {
        if self.wind_zones.len() >= MAX_WIND_ZONES {
            return Ok(ApplyResult::Capacity("wind zones"));
        }
        self.wind_zones.push(zone);
        Ok(match zone {
            WindZone::Constant(_) => ApplyResult::Applied,
            _ => ApplyResult::Placeholder(label),
        })
    }

    fn push_cloud(
        &mut self,
        cloud: ParticleCloud,
        placeholder: Option<&'static str>,
    ) -> Result<ApplyResult, String> {
        if self.particle_clouds.len() >= MAX_PARTICLE_CLOUDS {
            return Ok(ApplyResult::Capacity("particle clouds"));
        }
        self.particle_clouds.push(cloud);
        self.weather_authored = true;
        Ok(placeholder.map_or(ApplyResult::Applied, ApplyResult::Placeholder))
    }

    /// Current singular presentation choice used by the Weather settings page.
    /// If additive retail commands created multiple clouds, the last cloud is the
    /// presentation type while all clouds still remain active until the user picks
    /// a new singular type or issues `r_we clear`.
    pub fn weather_type(&self) -> WeatherType {
        self.particle_clouds.last().copied().map_or(WeatherType::Clear, |cloud| match cloud {
            ParticleCloud::LightRain => WeatherType::LightRain,
            ParticleCloud::Rain => WeatherType::Rain,
            ParticleCloud::AcidRain => WeatherType::AcidRain,
            ParticleCloud::HeavyRain => WeatherType::HeavyRain,
            ParticleCloud::Snow => WeatherType::Snow,
            ParticleCloud::SpaceDust(_) => WeatherType::SpaceDust,
            ParticleCloud::Sand => WeatherType::Sand,
            ParticleCloud::Fog => WeatherType::Fog,
            ParticleCloud::HeavyRainFog => WeatherType::HeavyRainFog,
            ParticleCloud::LightFog => WeatherType::LightFog,
        })
    }

    /// User-facing weather selection. Unlike the retail parser this is singular:
    /// replace particle-weather clouds, but do not destroy wind zones, weather
    /// zones, freeze state, or other world-effect compatibility state.
    pub fn set_weather_type(&mut self, weather: WeatherType) -> Result<ApplyResult, String> {
        self.particle_clouds.clear();
        self.weather_authored = true;
        let Some(command) = weather.command() else {
            return Ok(ApplyResult::Applied);
        };
        self.apply(command)
    }

    pub fn weather_authored(&self) -> bool {
        self.weather_authored
    }

    pub fn rain_override(&self) -> Option<RainOverride> {
        let rain: Vec<_> = self
            .particle_clouds
            .iter()
            .copied()
            .filter(|cloud| cloud.is_rain())
            .collect();
        let selected = rain.last().copied()?;
        let selected_legacy_count = selected.legacy_rain_particles().max(1);
        let total_legacy_count: u32 = rain.iter().map(|cloud| cloud.legacy_rain_particles()).sum();
        let intensity = match selected {
            ParticleCloud::LightRain => RainIntensity::Light,
            ParticleCloud::HeavyRain => RainIntensity::Heavy,
            ParticleCloud::Rain | ParticleCloud::AcidRain => RainIntensity::Rain,
            _ => unreachable!(),
        };
        Some(RainOverride {
            intensity,
            density_scale: total_legacy_count as f32 / selected_legacy_count as f32,
            tint: if selected == ParticleCloud::AcidRain {
                [0.34, 0.70, 0.34]
            } else {
                [1.0, 1.0, 1.0]
            },
            frozen: self.frozen,
        })
    }

    pub fn snow_override(&self) -> Option<SnowOverride> {
        let count = self
            .particle_clouds
            .iter()
            .filter(|cloud| matches!(cloud, ParticleCloud::Snow))
            .count();
        (count != 0).then_some(SnowOverride {
            density_scale: count as f32,
            frozen: self.frozen,
        })
    }

    /// Aggregate the legacy sand and space-dust clouds for the modern GPU
    /// atmospheric renderer. Multiple sand commands stack, while the argument
    /// to each spacedust command contributes to the requested mote count.
    pub fn atmosphere_override(&self) -> AtmosphereOverride {
        let mut sand_layers = 0u32;
        let mut space_dust_count = 0u32;
        for cloud in &self.particle_clouds {
            match *cloud {
                ParticleCloud::Sand => sand_layers = sand_layers.saturating_add(1),
                ParticleCloud::SpaceDust(count) if count > 0 => {
                    space_dust_count = space_dust_count.saturating_add(count as u32);
                }
                _ => {}
            }
        }
        AtmosphereOverride {
            sand_layers,
            space_dust_count,
            frozen: self.frozen,
        }
    }

    /// Match the rain override's "last rain cloud wins presentation" rule.
    /// Stacked rain still contributes density, but the last selected rain type
    /// controls whether the post-process atmosphere takes on the acid cast.
    pub fn acid_rain_strength(&self) -> f32 {
        if self
            .particle_clouds
            .iter()
            .rev()
            .copied()
            .find(|cloud| cloud.is_rain())
            == Some(ParticleCloud::AcidRain)
        {
            1.0
        } else {
            0.0
        }
    }

    /// Project only the part of retail wind that can be represented exactly by
    /// the current shared 2-D weather wind: the vector sum of constant zones.
    /// Random/gusting zones stay retained placeholders instead of being guessed.
    pub fn wind_override(&self) -> Option<AtmosphericWind> {
        let mut x = 0.0;
        let mut y = 0.0;
        let mut any = false;
        for zone in &self.wind_zones {
            if let WindZone::Constant([vx, vy, _]) = *zone {
                x += vx;
                y += vy;
                any = true;
            }
        }
        if !any {
            return None;
        }
        let speed = x.hypot(y);
        let direction = if speed > f32::EPSILON {
            y.atan2(x).to_degrees()
        } else {
            0.0
        };
        Some(AtmosphericWind {
            speed,
            direction,
            gust: 0.0,
            shift: 0.0,
        })
    }

    pub fn frozen(&self) -> bool { self.frozen }
    pub fn particle_clouds(&self) -> &[ParticleCloud] { &self.particle_clouds }
    pub fn wind_zones(&self) -> &[WindZone] { &self.wind_zones }
    pub fn weather_zones(&self) -> &[WeatherZone] { &self.weather_zones }
    pub fn outside_shake(&self) -> bool { self.outside_shake }
    pub fn outside_pain(&self) -> f32 { self.outside_pain }
}

/// Retail `WE_ParseVector` requires a literal parenthesized vector. Keep the
/// same token shape here; malformed `constantwind` uses the stock fallback.
fn parse_parenthesized_vec3<'a>(tokens: &mut impl Iterator<Item = &'a str>) -> Option<[f32; 3]> {
    if tokens.next()? != "(" {
        return None;
    }
    let mut output = [0.0; 3];
    for value in &mut output {
        *value = tokens.next()?.parse::<f32>().ok().filter(|v| v.is_finite())?;
    }
    (tokens.next()? == ")").then_some(output)
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rain_layers_share_the_retail_five_cloud_budget() {
        let mut world = LegacyWorldEffects::default();
        for _ in 0..5 {
            assert_eq!(world.apply("rain").unwrap(), ApplyResult::Applied);
        }
        assert_eq!(world.apply("snow").unwrap(), ApplyResult::Capacity("particle clouds"));
        assert_eq!(world.rain_override().unwrap().density_scale, 5.0);
    }

    #[test]
    fn snow_layers_use_stock_cloud_count_and_freeze_state() {
        let mut world = LegacyWorldEffects::default();
        assert_eq!(world.apply("snow").unwrap(), ApplyResult::Applied);
        assert_eq!(world.apply("snow").unwrap(), ApplyResult::Applied);
        let snow = world.snow_override().unwrap();
        assert_eq!(snow.density_scale, 2.0);
        assert!(!snow.frozen);
        world.apply("freeze").unwrap();
        assert!(world.snow_override().unwrap().frozen);
    }

    #[test]
    fn clear_preserves_retail_non_cloud_state_and_die_preserves_freeze_only() {
        let mut world = LegacyWorldEffects::default();
        world.apply("freeze").unwrap();
        world.apply("outsidepain").unwrap();
        world.apply("zone ( 0 1 2 ) ( 3 4 5 )").unwrap();
        world.apply("rain").unwrap();
        world.apply("constantwind ( 10 20 30 )").unwrap();
        world.apply("clear").unwrap();
        assert!(world.frozen());
        assert_eq!(world.outside_pain(), 1.0);
        assert_eq!(world.weather_zones().len(), 1);
        assert!(world.particle_clouds().is_empty());
        assert!(world.wind_zones().is_empty());
        world.apply("die").unwrap();
        assert!(world.frozen());
        assert_eq!(world.outside_pain(), 0.0);
        assert!(world.weather_zones().is_empty());
    }

    #[test]
    fn acid_rain_uses_existing_rain_path_with_stock_tint_and_pain() {
        let mut world = LegacyWorldEffects::default();
        world.apply("acidrain").unwrap();
        let rain = world.rain_override().unwrap();
        assert_eq!(rain.intensity, RainIntensity::Rain);
        assert_eq!(rain.tint, [0.34, 0.70, 0.34]);
        assert_eq!(world.outside_pain(), 0.1);
    }

    #[test]
    fn sand_and_spacedust_are_live_atmosphere_effects() {
        let mut world = LegacyWorldEffects::default();
        assert_eq!(world.apply("sand").unwrap(), ApplyResult::Applied);
        assert_eq!(world.apply("spacedust 700").unwrap(), ApplyResult::Applied);
        assert_eq!(world.apply("spacedust 300").unwrap(), ApplyResult::Applied);
        world.apply("freeze").unwrap();
        assert_eq!(
            world.atmosphere_override(),
            AtmosphereOverride {
                sand_layers: 1,
                space_dust_count: 1000,
                frozen: true,
            }
        );
    }

    #[test]
    fn last_rain_cloud_controls_acid_presentation() {
        let mut world = LegacyWorldEffects::default();
        world.apply("rain").unwrap();
        world.apply("acidrain").unwrap();
        assert_eq!(world.acid_rain_strength(), 1.0);
        world.apply("lightrain").unwrap();
        assert_eq!(world.acid_rain_strength(), 0.0);
    }

    #[test]
    fn clear_becomes_authoritative_even_when_no_clouds_remain() {
        let mut world = LegacyWorldEffects::default();
        assert!(!world.weather_authored());
        world.apply("clear").unwrap();
        assert!(world.weather_authored());
        assert_eq!(world.weather_type(), WeatherType::Clear);
        assert!(world.rain_override().is_none());
    }

    #[test]
    fn menu_weather_selection_replaces_clouds_but_preserves_wind() {
        let mut world = LegacyWorldEffects::default();
        world.apply("constantwind ( 100 200 0 )").unwrap();
        world.apply("rain").unwrap();
        world.apply("snow").unwrap();
        assert_eq!(world.particle_clouds().len(), 2);

        world.set_weather_type(WeatherType::AcidRain).unwrap();
        assert_eq!(world.particle_clouds(), &[ParticleCloud::AcidRain]);
        assert_eq!(world.wind_zones().len(), 1);
        assert_eq!(world.weather_type(), WeatherType::AcidRain);

        world.set_weather_type(WeatherType::Clear).unwrap();
        assert!(world.particle_clouds().is_empty());
        assert_eq!(world.wind_zones().len(), 1);
        assert_eq!(world.weather_type(), WeatherType::Clear);
    }
}
