// =============================================================================
// lib — Core library for the multicall binary.
// =============================================================================
// Copyright (c) 2026 Semyon Tsarev
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// project, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Implementation inspired by Toybox (https://landley.net/toybox/)
// Toybox is copyrighted by Rob Landley, see NOTICE file for license details.
//
// Re-exports the public modules that comprise the command framework.
// =============================================================================

pub mod args;
pub mod commands;
pub mod context;
pub mod flags;
pub mod registry;
pub mod sh;
