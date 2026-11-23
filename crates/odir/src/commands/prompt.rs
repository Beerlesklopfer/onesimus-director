// SPDX-License-Identifier: AGPL-3.0-only
// SPDX-FileCopyrightText: 2000-2012 Free Software Foundation Europe e.V.
// SPDX-FileCopyrightText: 2010-2017 Planets Communications B.V.
// SPDX-FileCopyrightText: 2012-2026 Bareos GmbH & Co. KG
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! Console helpers of the commands. The interactive dialogs themselves
//! (StartPrompt/DoPrompt, GetCmd, GetYesno, GetPint with BNET_SUB_PROMPT)
//! live in [`crate::ua_select`] (compatibility log C-011).

/// `SendCmdUsage` (ua_output.cc:1923): "<message>\nUSAGE: <cmd> <usage>\n".
pub fn usage(command: &str, message: &str) -> String {
    let usage = crate::ua_commands_table::UA_COMMANDS.iter()
        .find(|entry| entry.0 == command).map(|entry| entry.2).unwrap_or("");
    format!("{message}\nUSAGE: {command} {usage}\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_uses_original_table() {
        assert_eq!(usage("truncate", "missing parameter"), "missing parameter\nUSAGE: truncate volstatus=Purged [storage=<storage>] [pool=<pool>] [volume=<volume>] [drive=<drivenum>] [yes]\n");
    }
}
