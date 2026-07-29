use std::{fs, io, path::Path};

use crate::{
    error::{Error, ErrorKind},
    model::ControllerModel,
    profile::{PairingProfile, ProfileDocument},
};

pub(super) use crate::profile::ProfileReadPort;

pub(super) struct FileProfileReader;

impl ProfileReadPort for FileProfileReader {
    fn read(&mut self, path: &Path) -> io::Result<Vec<u8>> {
        fs::read(path)
    }
}

pub(super) fn read_typed_profile<M: ControllerModel>(
    reader: &mut impl ProfileReadPort,
    path: &Path,
) -> crate::Result<PairingProfile<M>> {
    let bytes = reader.read(path).map_err(|source| {
        let (kind, message) = if source.kind() == io::ErrorKind::NotFound {
            (ErrorKind::ProfileNotFound, "pairing profile was not found")
        } else {
            (ErrorKind::Internal, "pairing profile could not be read")
        };

        Error::with_source(kind, message, source)
    })?;
    let document = ProfileDocument::parse_json(&bytes)?;

    PairingProfile::try_from(document)
}
