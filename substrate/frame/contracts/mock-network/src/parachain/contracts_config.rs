// Copyright (C) Parity Technologies (UK) Ltd.
// This file is part of Substrate.

// Substrate is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

// Substrate is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.

// You should have received a copy of the GNU General Public License
// along with Substrate.  If not, see <http://www.gnu.org/licenses/>.

use super::{Balances, Runtime, RuntimeCall, RuntimeEvent};
use crate::parachain::RuntimeHoldReason;
use frame_support::{derive_impl, parameter_types};

parameter_types! {
	pub Schedule: pallet_contracts::Schedule<Runtime> = Default::default();
}

#[derive_impl(pallet_contracts::config_preludes::TestDefaultConfig)]
impl pallet_contracts::Config for Runtime {
	type AddressGenerator = pallet_contracts::DefaultAddressGenerator;
	type CallStack = [pallet_contracts::Frame<Self>; 5];
	type Currency = Balances;
	type Schedule = Schedule;
	type Time = super::Timestamp;
	type Xcm = pallet_xcm::Pallet<Self>;
}
