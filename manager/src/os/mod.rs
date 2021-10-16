use std::convert::{TryFrom, TryInto};
use std::fmt::Display;

use anyhow::{bail, Error};

#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "linux")]
pub use linux::ContainerManager;

#[derive(Debug, Copy, Clone, Eq, PartialEq, Ord, PartialOrd)]
pub enum ChildState {
    None,
    Created,
    Started,
    Stopped,
    Crashed(isize),
}

impl TryFrom<u8> for ChildState {
    type Error = Error;

    fn try_from(value: u8) -> std::result::Result<Self, Self::Error> {
        Ok(match value {
            0 => ChildState::None,
            1 => ChildState::Created,
            2 => ChildState::Started,
            3 => ChildState::Stopped,
            _ => bail!("Invalid ChildState"),
        })
    }
}

impl TryFrom<isize> for ChildState {
    type Error = Error;

    fn try_from(value: isize) -> std::result::Result<Self, Self::Error> {
        let state = value & 0b1111;
        Ok(if state == 4 {
            let status = value >> 4;
            ChildState::Crashed(status)
        } else {
            (state as u8).try_into()?
        })
    }
}

impl TryFrom<ChildState> for u8 {
    type Error = Error;

    fn try_from(value: ChildState) -> std::result::Result<Self, Self::Error> {
        Ok(match value {
            ChildState::None => 0,
            ChildState::Created => 1,
            ChildState::Started => 2,
            ChildState::Stopped => 3,
            v => bail!("Cannot transform {:?} to u8"),
        })
    }
}

impl From<ChildState> for isize {
    fn from(value: ChildState) -> Self {
        if let ChildState::Crashed(status) = value {
            (status << 4) + 4
        } else {
            u8::try_from(value).unwrap_or(0) as isize
        }
    }
}

impl Display for ChildState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}
#[cfg(test)]
mod test {
    use crate::os::ChildState;
    use std::convert::TryFrom;

    #[test]
    fn childstate_to_u8() {
        assert_eq!(u8::try_from(ChildState::None).unwrap(), 0);
        assert_eq!(u8::try_from(ChildState::Created).unwrap(), 1);
        assert_eq!(u8::try_from(ChildState::Started).unwrap(), 2);
        assert_eq!(u8::try_from(ChildState::Stopped).unwrap(), 3);
        assert!(u8::try_from(ChildState::Crashed(0)).is_err());
    }

    #[test]
    fn childstate_from_u8() {
        assert_eq!(ChildState::try_from(0u8).unwrap(), ChildState::None);
        assert_eq!(ChildState::try_from(1u8).unwrap(), ChildState::Created);
        assert_eq!(ChildState::try_from(2u8).unwrap(), ChildState::Started);
        assert_eq!(ChildState::try_from(3u8).unwrap(), ChildState::Stopped);
        assert!(ChildState::try_from(4u8).is_err());
    }

    #[test]
    fn childstate_to_isize() {
        assert_eq!(isize::try_from(ChildState::None).unwrap(), 0);
        assert_eq!(isize::try_from(ChildState::Created).unwrap(), 1);
        assert_eq!(isize::try_from(ChildState::Started).unwrap(), 2);
        assert_eq!(isize::try_from(ChildState::Stopped).unwrap(), 3);
        assert_eq!(isize::try_from(ChildState::Crashed(0)).unwrap(), 4);

        assert_eq!(
            isize::try_from(ChildState::Crashed(-1)).unwrap(),
            ((-1 as isize) << 4) + 4
        );
        assert_eq!(
            isize::try_from(ChildState::Crashed(10)).unwrap(),
            ((10 as isize) << 4) + 4
        );
    }

    #[test]
    fn childstate_from_isize() {
        assert_eq!(ChildState::try_from(0isize).unwrap(), ChildState::None);
        assert_eq!(ChildState::try_from(1isize).unwrap(), ChildState::Created);
        assert_eq!(ChildState::try_from(2isize).unwrap(), ChildState::Started);
        assert_eq!(ChildState::try_from(3isize).unwrap(), ChildState::Stopped);
        assert_eq!(
            ChildState::try_from(4isize).unwrap(),
            ChildState::Crashed(0)
        );

        assert_eq!(
            ChildState::try_from(((-1 as isize) << 4) + 4).unwrap(),
            ChildState::Crashed(-1)
        );
        assert_eq!(
            ChildState::try_from(((10 as isize) << 4) + 4).unwrap(),
            ChildState::Crashed(10)
        );
    }
}
