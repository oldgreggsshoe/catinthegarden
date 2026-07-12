use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct ScenarioDefinition {
    pub name: String,
    pub fixed_timestep_seconds: f64,
    pub duration_seconds: f64,
    pub solid_color_screen: bool,
    pub screenshot_times_seconds: Vec<f64>,
    pub waypoints: Vec<Waypoint>,
}

#[derive(Debug, Deserialize)]
pub struct Waypoint {
    pub time_s: f64,
    pub position: [f64; 3],
    pub look_at: [f64; 3],
}

pub struct FramePlan {
    pub sim_time: f64,
    pub write_log: bool,
    pub capture_screenshot: bool,
    pub complete: bool,
}

pub struct ScenarioRunner {
    definition: ScenarioDefinition,
    sim_time: f64,
    next_screenshot: usize,
    next_log_time: f64,
}

impl ScenarioRunner {
    pub fn load(name: &str) -> Result<Self, String> {
        let source = match name {
            "still_5s" => include_str!("../scenarios/still_5s.json"),
            _ => return Err(format!("unknown scenario '{name}'")),
        };
        let definition: ScenarioDefinition =
            serde_json::from_str(source).map_err(|error| error.to_string())?;
        if definition.fixed_timestep_seconds <= 0.0 || definition.duration_seconds <= 0.0 {
            return Err("scenario timings must be positive".to_owned());
        }
        if definition
            .screenshot_times_seconds
            .iter()
            .any(|time| *time <= 0.0 || *time > definition.duration_seconds)
        {
            return Err("screenshot times must be within the scenario duration".to_owned());
        }
        if definition.waypoints.is_empty()
            || definition.waypoints.iter().any(|waypoint| {
                !waypoint.time_s.is_finite()
                    || waypoint.position.iter().any(|value| !value.is_finite())
                    || waypoint.look_at.iter().any(|value| !value.is_finite())
            })
        {
            return Err("scenario waypoints must be present and finite".to_owned());
        }

        Ok(Self {
            definition,
            sim_time: 0.0,
            next_screenshot: 0,
            next_log_time: 0.0,
        })
    }

    pub fn name(&self) -> &str {
        &self.definition.name
    }

    pub fn renders_solid_color(&self) -> bool {
        self.definition.solid_color_screen
    }

    pub fn expected_screenshots(&self) -> usize {
        self.definition.screenshot_times_seconds.len()
    }

    pub fn advance(&mut self) -> FramePlan {
        self.sim_time = (self.sim_time + self.definition.fixed_timestep_seconds)
            .min(self.definition.duration_seconds);

        let write_log = self.sim_time + f64::EPSILON >= self.next_log_time;
        if write_log {
            self.next_log_time += 0.5;
        }

        let capture_screenshot = self
            .definition
            .screenshot_times_seconds
            .get(self.next_screenshot)
            .is_some_and(|time| self.sim_time + f64::EPSILON >= *time);
        if capture_screenshot {
            self.next_screenshot += 1;
        }

        FramePlan {
            sim_time: self.sim_time,
            write_log,
            capture_screenshot,
            complete: self.sim_time + f64::EPSILON >= self.definition.duration_seconds
                && self.next_screenshot == self.definition.screenshot_times_seconds.len(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ScenarioRunner;

    #[test]
    fn still_scenario_has_three_deterministic_captures() {
        let mut scenario = ScenarioRunner::load("still_5s").expect("scenario parses");
        let mut captures = 0;
        let completion_time = loop {
            let frame = scenario.advance();
            captures += usize::from(frame.capture_screenshot);
            if frame.complete {
                break frame.sim_time;
            }
        };

        assert_eq!(captures, 3);
        assert_eq!(completion_time, 5.0);
    }
}
