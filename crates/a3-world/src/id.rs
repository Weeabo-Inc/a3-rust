//! Identifiers: Entity ID (ours), Network object ID and client ID (the original's).

use std::fmt;
use std::str::FromStr;

/// A machine in the session, by its player id (dpnid). The server is [`ClientId::SERVER`];
/// clients get large ids from the connect handshake.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ClientId(pub u32);

impl ClientId {
    /// The server's own client: the first local id the original's `NetworkServer` hands out.
    pub const SERVER: ClientId = ClientId(2);
}

/// The original engine's Network object ID: the creating machine's client id and that
/// machine's serial number. The same on every machine; `netId` prints it as `"creator:id"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NetworkId {
    pub creator: u32,
    pub id: u32,
}

impl NetworkId {
    /// No object (`creator` 0).
    pub const NULL: NetworkId = NetworkId { creator: 0, id: 0 };
    /// The creator reserved for Static objects.
    pub const STATIC_CREATOR: u32 = 1;

    pub const fn new(creator: u32, id: u32) -> Self {
        Self { creator, id }
    }

    /// Creator 0: refers to nothing.
    pub const fn is_null(self) -> bool {
        self.creator == 0
    }

    /// Creator 1: a Static object, resolved through the terrain rather than a create message.
    pub const fn is_static(self) -> bool {
        self.creator == Self::STATIC_CREATOR
    }
}

impl fmt::Display for NetworkId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The original formats both halves with `%d`.
        write!(f, "{}:{}", self.creator as i32, self.id as i32)
    }
}

/// The text is not `"creator:id"` with two integers.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("not a network id: {0:?}")]
pub struct ParseNetworkIdError(pub String);

impl FromStr for NetworkId {
    type Err = ParseNetworkIdError;

    /// Parses `"creator:id"` (each half a signed 32-bit decimal, as `objectFromNetId` accepts).
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let err = || ParseNetworkIdError(s.to_owned());
        let (creator, id) = s.split_once(':').ok_or_else(err)?;
        let creator: i32 = creator.trim().parse().map_err(|_| err())?;
        let id: i32 = id.trim().parse().map_err(|_| err())?;
        Ok(NetworkId::new(creator as u32, id as u32))
    }
}

/// Our handle to an Entity in a [`World`](crate::World). A deleted Entity's ID never refers to
/// another Entity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EntityId {
    pub(crate) index: u32,
    pub(crate) generation: u32,
}
