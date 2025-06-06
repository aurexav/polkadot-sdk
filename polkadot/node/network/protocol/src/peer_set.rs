// Copyright (C) Parity Technologies (UK) Ltd.
// This file is part of Polkadot.

// Polkadot is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

// Polkadot is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.

// You should have received a copy of the GNU General Public License
// along with Polkadot.  If not, see <http://www.gnu.org/licenses/>.

//! All peersets and protocols used for parachains.

use derive_more::Display;
use polkadot_primitives::Hash;
use sc_network::{
	config::SetConfig, peer_store::PeerStoreProvider, service::NotificationMetrics,
	types::ProtocolName, NetworkBackend, NotificationService,
};
use sp_runtime::traits::Block;
use std::{
	collections::{hash_map::Entry, HashMap},
	ops::{Index, IndexMut},
	sync::Arc,
};
use strum::{EnumIter, IntoEnumIterator};

/// The legacy collation protocol name. Only supported on version = 1.
const LEGACY_COLLATION_PROTOCOL_V1: &str = "/polkadot/collation/1";

/// The legacy protocol version. Is always 1 for collation.
const LEGACY_COLLATION_PROTOCOL_VERSION_V1: u32 = 1;

/// Max notification size is currently constant.
pub const MAX_NOTIFICATION_SIZE: u64 = 100 * 1024;

/// The peer-sets and thus the protocols which are used for the network.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, EnumIter)]
pub enum PeerSet {
	/// The validation peer-set is responsible for all messages related to candidate validation and
	/// communication among validators.
	Validation,
	/// The collation peer-set is used for validator<>collator communication.
	Collation,
}

/// Whether a node is an authority or not.
///
/// Peer set configuration gets adjusted accordingly.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum IsAuthority {
	/// Node is authority.
	Yes,
	/// Node is not an authority.
	No,
}

impl PeerSet {
	/// Get `sc_network` peer set configurations for each peerset on the default version.
	///
	/// Those should be used in the network configuration to register the protocols with the
	/// network service.
	pub fn get_info<B: Block, N: NetworkBackend<B, <B as Block>::Hash>>(
		self,
		is_authority: IsAuthority,
		peerset_protocol_names: &PeerSetProtocolNames,
		metrics: NotificationMetrics,
		peer_store_handle: Arc<dyn PeerStoreProvider>,
	) -> (N::NotificationProtocolConfig, (PeerSet, Box<dyn NotificationService>)) {
		// Networking layer relies on `get_main_name()` being the main name of the protocol
		// for peersets and connection management.
		let protocol = peerset_protocol_names.get_main_name(self);
		let fallback_names = PeerSetProtocolNames::get_fallback_names(
			self,
			&peerset_protocol_names.genesis_hash,
			peerset_protocol_names.fork_id.as_deref(),
		);
		let max_notification_size = self.get_max_notification_size(is_authority);

		match self {
			PeerSet::Validation => {
				let (config, notification_service) = N::notification_config(
					protocol,
					fallback_names,
					max_notification_size,
					None,
					SetConfig {
						// we allow full nodes to connect to validators for gossip
						// to ensure any `MIN_GOSSIP_PEERS` always include reserved peers
						// we limit the amount of non-reserved slots to be less
						// than `MIN_GOSSIP_PEERS` in total
						in_peers: super::MIN_GOSSIP_PEERS as u32 / 2 - 1,
						out_peers: super::MIN_GOSSIP_PEERS as u32 / 2 - 1,
						reserved_nodes: Vec::new(),
						non_reserved_mode: sc_network::config::NonReservedPeerMode::Accept,
					},
					metrics,
					peer_store_handle,
				);

				(config, (PeerSet::Validation, notification_service))
			},
			PeerSet::Collation => {
				let (config, notification_service) = N::notification_config(
					protocol,
					fallback_names,
					max_notification_size,
					None,
					SetConfig {
						// Non-authority nodes don't need to accept incoming connections on this
						// peer set:
						in_peers: if is_authority == IsAuthority::Yes { 100 } else { 0 },
						out_peers: 0,
						reserved_nodes: Vec::new(),
						non_reserved_mode: if is_authority == IsAuthority::Yes {
							sc_network::config::NonReservedPeerMode::Accept
						} else {
							sc_network::config::NonReservedPeerMode::Deny
						},
					},
					metrics,
					peer_store_handle,
				);

				(config, (PeerSet::Collation, notification_service))
			},
		}
	}

	/// Get the main protocol version for this peer set.
	///
	/// Networking layer relies on `get_main_version()` being the version
	/// of the main protocol name reported by [`PeerSetProtocolNames::get_main_name()`].
	pub fn get_main_version(self) -> ProtocolVersion {
		match self {
			PeerSet::Validation => ValidationVersion::V3.into(),
			PeerSet::Collation => CollationVersion::V2.into(),
		}
	}

	/// Get the max notification size for this peer set.
	pub fn get_max_notification_size(self, _: IsAuthority) -> u64 {
		MAX_NOTIFICATION_SIZE
	}

	/// Get the peer set label for metrics reporting.
	pub fn get_label(self) -> &'static str {
		match self {
			PeerSet::Validation => "validation",
			PeerSet::Collation => "collation",
		}
	}

	/// Get the protocol label for metrics reporting.
	pub fn get_protocol_label(self, version: ProtocolVersion) -> Option<&'static str> {
		// Unfortunately, labels must be static strings, so we must manually cover them
		// for all protocol versions here.
		match self {
			PeerSet::Validation =>
				if version == ValidationVersion::V3.into() {
					Some("validation/3")
				} else {
					None
				},
			PeerSet::Collation =>
				if version == CollationVersion::V1.into() {
					Some("collation/1")
				} else if version == CollationVersion::V2.into() {
					Some("collation/2")
				} else {
					None
				},
		}
	}
}

/// A small and nifty collection that allows to store data pertaining to each peer set.
#[derive(Debug, Default)]
pub struct PerPeerSet<T> {
	validation: T,
	collation: T,
}

impl<T> Index<PeerSet> for PerPeerSet<T> {
	type Output = T;
	fn index(&self, index: PeerSet) -> &T {
		match index {
			PeerSet::Validation => &self.validation,
			PeerSet::Collation => &self.collation,
		}
	}
}

impl<T> IndexMut<PeerSet> for PerPeerSet<T> {
	fn index_mut(&mut self, index: PeerSet) -> &mut T {
		match index {
			PeerSet::Validation => &mut self.validation,
			PeerSet::Collation => &mut self.collation,
		}
	}
}

/// Get `NonDefaultSetConfig`s for all available peer sets, at their default versions.
///
/// Should be used during network configuration (added to `NetworkConfiguration::extra_sets`)
/// or shortly after startup to register the protocols with the network service.
pub fn peer_sets_info<B: Block, N: NetworkBackend<B, <B as Block>::Hash>>(
	is_authority: IsAuthority,
	peerset_protocol_names: &PeerSetProtocolNames,
	metrics: NotificationMetrics,
	peer_store_handle: Arc<dyn PeerStoreProvider>,
) -> Vec<(N::NotificationProtocolConfig, (PeerSet, Box<dyn NotificationService>))> {
	PeerSet::iter()
		.map(|s| {
			s.get_info::<B, N>(
				is_authority,
				&peerset_protocol_names,
				metrics.clone(),
				Arc::clone(&peer_store_handle),
			)
		})
		.collect()
}

/// A generic version of the protocol. This struct must not be created directly.
#[derive(Debug, Clone, Copy, Display, PartialEq, Eq, Hash)]
pub struct ProtocolVersion(u32);

impl From<ProtocolVersion> for u32 {
	fn from(version: ProtocolVersion) -> u32 {
		version.0
	}
}

/// Supported validation protocol versions. Only versions defined here must be used in the codebase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, EnumIter)]
pub enum ValidationVersion {
	/// The third version.
	V3 = 3,
}

/// Supported collation protocol versions. Only versions defined here must be used in the codebase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, EnumIter)]
pub enum CollationVersion {
	/// The first version.
	V1 = 1,
	/// The second version.
	V2 = 2,
}

/// Marker indicating the version is unknown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnknownVersion;

impl TryFrom<ProtocolVersion> for ValidationVersion {
	type Error = UnknownVersion;

	fn try_from(p: ProtocolVersion) -> Result<Self, UnknownVersion> {
		for v in Self::iter() {
			if v as u32 == p.0 {
				return Ok(v)
			}
		}

		Err(UnknownVersion)
	}
}

impl TryFrom<ProtocolVersion> for CollationVersion {
	type Error = UnknownVersion;

	fn try_from(p: ProtocolVersion) -> Result<Self, UnknownVersion> {
		for v in Self::iter() {
			if v as u32 == p.0 {
				return Ok(v)
			}
		}

		Err(UnknownVersion)
	}
}

impl From<ValidationVersion> for ProtocolVersion {
	fn from(version: ValidationVersion) -> ProtocolVersion {
		ProtocolVersion(version as u32)
	}
}

impl From<CollationVersion> for ProtocolVersion {
	fn from(version: CollationVersion) -> ProtocolVersion {
		ProtocolVersion(version as u32)
	}
}

/// On the wire protocol name to [`PeerSet`] mapping.
#[derive(Debug, Clone)]
pub struct PeerSetProtocolNames {
	protocols: HashMap<ProtocolName, (PeerSet, ProtocolVersion)>,
	names: HashMap<(PeerSet, ProtocolVersion), ProtocolName>,
	genesis_hash: Hash,
	fork_id: Option<String>,
}

impl PeerSetProtocolNames {
	/// Construct [`PeerSetProtocolNames`] using `genesis_hash` and `fork_id`.
	pub fn new(genesis_hash: Hash, fork_id: Option<&str>) -> Self {
		let mut protocols = HashMap::new();
		let mut names = HashMap::new();
		for protocol in PeerSet::iter() {
			match protocol {
				PeerSet::Validation =>
					for version in ValidationVersion::iter() {
						Self::register_main_protocol(
							&mut protocols,
							&mut names,
							protocol,
							version.into(),
							&genesis_hash,
							fork_id,
						);
					},
				PeerSet::Collation => {
					for version in CollationVersion::iter() {
						Self::register_main_protocol(
							&mut protocols,
							&mut names,
							protocol,
							version.into(),
							&genesis_hash,
							fork_id,
						);
					}
					Self::register_legacy_collation_protocol(&mut protocols, protocol);
				},
			}
		}
		Self { protocols, names, genesis_hash, fork_id: fork_id.map(|fork_id| fork_id.into()) }
	}

	/// Helper function to register main protocol.
	fn register_main_protocol(
		protocols: &mut HashMap<ProtocolName, (PeerSet, ProtocolVersion)>,
		names: &mut HashMap<(PeerSet, ProtocolVersion), ProtocolName>,
		protocol: PeerSet,
		version: ProtocolVersion,
		genesis_hash: &Hash,
		fork_id: Option<&str>,
	) {
		let protocol_name = Self::generate_name(genesis_hash, fork_id, protocol, version);
		names.insert((protocol, version), protocol_name.clone());
		Self::insert_protocol_or_panic(protocols, protocol_name, protocol, version);
	}

	/// Helper function to register legacy collation protocol.
	fn register_legacy_collation_protocol(
		protocols: &mut HashMap<ProtocolName, (PeerSet, ProtocolVersion)>,
		protocol: PeerSet,
	) {
		Self::insert_protocol_or_panic(
			protocols,
			LEGACY_COLLATION_PROTOCOL_V1.into(),
			protocol,
			ProtocolVersion(LEGACY_COLLATION_PROTOCOL_VERSION_V1),
		)
	}

	/// Helper function to make sure no protocols have the same name.
	fn insert_protocol_or_panic(
		protocols: &mut HashMap<ProtocolName, (PeerSet, ProtocolVersion)>,
		name: ProtocolName,
		protocol: PeerSet,
		version: ProtocolVersion,
	) {
		match protocols.entry(name) {
			Entry::Vacant(entry) => {
				entry.insert((protocol, version));
			},
			Entry::Occupied(entry) => {
				panic!(
					"Protocol {:?} (version {}) has the same on-the-wire name as protocol {:?} (version {}): `{}`.",
					protocol,
					version,
					entry.get().0,
					entry.get().1,
					entry.key(),
				);
			},
		}
	}

	/// Lookup the protocol using its on the wire name.
	pub fn try_get_protocol(&self, name: &ProtocolName) -> Option<(PeerSet, ProtocolVersion)> {
		self.protocols.get(name).map(ToOwned::to_owned)
	}

	/// Get the main protocol name. It's used by the networking for keeping track
	/// of peersets and connections.
	pub fn get_main_name(&self, protocol: PeerSet) -> ProtocolName {
		self.get_name(protocol, protocol.get_main_version())
	}

	/// Get the protocol name for specific version.
	pub fn get_name(&self, protocol: PeerSet, version: ProtocolVersion) -> ProtocolName {
		self.names
			.get(&(protocol, version))
			.expect("Protocols & versions are specified via enums defined above, and they are all registered in `new()`; qed")
			.clone()
	}

	/// The protocol name of this protocol based on `genesis_hash` and `fork_id`.
	fn generate_name(
		genesis_hash: &Hash,
		fork_id: Option<&str>,
		protocol: PeerSet,
		version: ProtocolVersion,
	) -> ProtocolName {
		let prefix = if let Some(fork_id) = fork_id {
			format!("/{}/{}", hex::encode(genesis_hash), fork_id)
		} else {
			format!("/{}", hex::encode(genesis_hash))
		};

		let short_name = match protocol {
			PeerSet::Validation => "validation",
			PeerSet::Collation => "collation",
		};

		format!("{}/{}/{}", prefix, short_name, version).into()
	}

	/// Get the protocol fallback names. Currently, it only holds
	/// the legacy name for the collation protocol version 1.
	fn get_fallback_names(
		protocol: PeerSet,
		_genesis_hash: &Hash,
		_fork_id: Option<&str>,
	) -> Vec<ProtocolName> {
		let mut fallbacks = vec![];
		match protocol {
			PeerSet::Validation => {
				// The validation protocol no longer supports protocol versions 1 and 2,
				// and only version 3 is used. Therefore, fallback protocols remain empty.
			},
			PeerSet::Collation => {
				fallbacks.push(LEGACY_COLLATION_PROTOCOL_V1.into());
			},
		};
		fallbacks
	}
}

#[cfg(test)]
mod tests {
	use super::{
		CollationVersion, Hash, PeerSet, PeerSetProtocolNames, ProtocolVersion, ValidationVersion,
	};
	use strum::IntoEnumIterator;

	struct TestVersion(u32);

	impl From<TestVersion> for ProtocolVersion {
		fn from(version: TestVersion) -> ProtocolVersion {
			ProtocolVersion(version.0)
		}
	}

	#[test]
	fn protocol_names_are_correctly_generated() {
		let genesis_hash = Hash::from([
			122, 200, 116, 29, 232, 183, 20, 109, 138, 86, 23, 253, 70, 41, 20, 85, 127, 230, 60,
			38, 90, 127, 28, 16, 231, 218, 227, 40, 88, 238, 187, 128,
		]);
		let name = PeerSetProtocolNames::generate_name(
			&genesis_hash,
			None,
			PeerSet::Validation,
			TestVersion(3).into(),
		);
		let expected =
			"/7ac8741de8b7146d8a5617fd462914557fe63c265a7f1c10e7dae32858eebb80/validation/3";
		assert_eq!(name, expected.into());

		let name = PeerSetProtocolNames::generate_name(
			&genesis_hash,
			None,
			PeerSet::Collation,
			TestVersion(5).into(),
		);
		let expected =
			"/7ac8741de8b7146d8a5617fd462914557fe63c265a7f1c10e7dae32858eebb80/collation/5";
		assert_eq!(name, expected.into());

		let fork_id = Some("test-fork");
		let name = PeerSetProtocolNames::generate_name(
			&genesis_hash,
			fork_id,
			PeerSet::Validation,
			TestVersion(7).into(),
		);
		let expected =
			"/7ac8741de8b7146d8a5617fd462914557fe63c265a7f1c10e7dae32858eebb80/test-fork/validation/7";
		assert_eq!(name, expected.into());

		let name = PeerSetProtocolNames::generate_name(
			&genesis_hash,
			fork_id,
			PeerSet::Collation,
			TestVersion(11).into(),
		);
		let expected =
			"/7ac8741de8b7146d8a5617fd462914557fe63c265a7f1c10e7dae32858eebb80/test-fork/collation/11";
		assert_eq!(name, expected.into());
	}

	#[test]
	fn all_protocol_names_are_known() {
		let genesis_hash = Hash::from([
			122, 200, 116, 29, 232, 183, 20, 109, 138, 86, 23, 253, 70, 41, 20, 85, 127, 230, 60,
			38, 90, 127, 28, 16, 231, 218, 227, 40, 88, 238, 187, 128,
		]);
		let protocol_names = PeerSetProtocolNames::new(genesis_hash, None);

		let validation_main =
			"/7ac8741de8b7146d8a5617fd462914557fe63c265a7f1c10e7dae32858eebb80/validation/3";
		assert_eq!(
			protocol_names.try_get_protocol(&validation_main.into()),
			Some((PeerSet::Validation, TestVersion(3).into())),
		);

		let validation_legacy = "/polkadot/validation/1";
		assert!(protocol_names.try_get_protocol(&validation_legacy.into()).is_none());

		let collation_main =
			"/7ac8741de8b7146d8a5617fd462914557fe63c265a7f1c10e7dae32858eebb80/collation/1";
		assert_eq!(
			protocol_names.try_get_protocol(&collation_main.into()),
			Some((PeerSet::Collation, TestVersion(1).into())),
		);

		let collation_legacy = "/polkadot/collation/1";
		assert_eq!(
			protocol_names.try_get_protocol(&collation_legacy.into()),
			Some((PeerSet::Collation, TestVersion(1).into())),
		);
	}

	#[test]
	fn all_protocol_versions_are_registered() {
		let genesis_hash = Hash::from([
			122, 200, 116, 29, 232, 183, 20, 109, 138, 86, 23, 253, 70, 41, 20, 85, 127, 230, 60,
			38, 90, 127, 28, 16, 231, 218, 227, 40, 88, 238, 187, 128,
		]);
		let protocol_names = PeerSetProtocolNames::new(genesis_hash, None);

		for protocol in PeerSet::iter() {
			match protocol {
				PeerSet::Validation =>
					for version in ValidationVersion::iter() {
						assert_eq!(
							protocol_names.get_name(protocol, version.into()),
							PeerSetProtocolNames::generate_name(
								&genesis_hash,
								None,
								protocol,
								version.into(),
							),
						);
					},
				PeerSet::Collation =>
					for version in CollationVersion::iter() {
						assert_eq!(
							protocol_names.get_name(protocol, version.into()),
							PeerSetProtocolNames::generate_name(
								&genesis_hash,
								None,
								protocol,
								version.into(),
							),
						);
					},
			}
		}
	}

	#[test]
	fn all_protocol_versions_have_labels() {
		for protocol in PeerSet::iter() {
			match protocol {
				PeerSet::Validation =>
					for version in ValidationVersion::iter() {
						protocol
							.get_protocol_label(version.into())
							.expect("All validation protocol versions must have a label.");
					},
				PeerSet::Collation =>
					for version in CollationVersion::iter() {
						protocol
							.get_protocol_label(version.into())
							.expect("All collation protocol versions must have a label.");
					},
			}
		}
	}
}
