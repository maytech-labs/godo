// SPDX-License-Identifier: LicenseRef-REAL-1.0

use clapsmith::{Dispatch, Parser};

#[derive(Parser, Dispatch)]
enum Cli {
    Status,
}

fn main() {}
