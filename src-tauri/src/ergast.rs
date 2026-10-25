//! The bits of Jolpica's JSON that several answers share: a driver and a constructor
//! (a team). Jolpica speaks the "Ergast" format, hence the name.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize)]
pub struct Driver {
    pub code: Option<String>,
    #[serde(rename = "familyName")]
    pub family_name: String,
}

impl Driver {
    /// The three-letter code ("VER"). Older drivers have none in the data, so the first
    /// three letters of the family name, in capitals, stand in.
    pub fn short_code(&self) -> String {
        match &self.code {
            Some(code) => code.clone(),
            None => self
                .family_name
                .chars()
                .take(3)
                .collect::<String>()
                .to_uppercase(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Constructor {
    #[serde(rename = "constructorId")]
    pub id: String,
    pub name: String,
}

/// A place on a podium: used by the last race and by a circuit's previous race.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PodiumEntry {
    pub pos: String,
    pub code: String,
    pub team_id: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn driver(code: Option<&str>, family: &str) -> Driver {
        Driver {
            code: code.map(String::from),
            family_name: family.into(),
        }
    }

    #[test]
    fn uses_the_code_when_there_is_one() {
        assert_eq!(driver(Some("VER"), "Verstappen").short_code(), "VER");
    }

    #[test]
    fn falls_back_to_the_first_three_letters_of_the_name() {
        assert_eq!(driver(None, "Fangio").short_code(), "FAN");
        assert_eq!(driver(None, "Ng").short_code(), "NG"); // shorter than three letters
        assert_eq!(driver(None, "ékman").short_code(), "ÉKM"); // non-ASCII letters survive
    }
}
