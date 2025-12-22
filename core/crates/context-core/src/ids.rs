use serde::{Deserialize, Serialize};
use uuid::Uuid;

macro_rules! id_type {
    ($name:ident) => {
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord,
        )]
        pub struct $name(pub Uuid);

        impl $name {
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }
        }

        impl From<Uuid> for $name {
            fn from(value: Uuid) -> Self {
                Self(value)
            }
        }

        impl From<$name> for Uuid {
            fn from(value: $name) -> Self {
                value.0
            }
        }
    };
}

id_type!(WorkspaceId);
id_type!(TaskId);
id_type!(TrackId);
id_type!(WorktreeId);
id_type!(SessionId);
id_type!(MessageId);
id_type!(SessionEventId);
id_type!(RunId);
id_type!(TurnId);
id_type!(ConnectionProfileId);
id_type!(MobileDeviceId);
