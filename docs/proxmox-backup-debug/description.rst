Implements debugging functionality to inspect Proxmox Backup datastore
files, verify the integrity of chunks.

The 'diff' subcommand allows comparing .pxar archives for two
arbitrary snapshots. A list of added/modified/deleted files will be displayed.

Also contains an 'api' subcommand where arbitrary api paths can be called
(get/create/set/delete) as well as display their parameters (usage) and
their child-links (ls).

By default, it connects to the proxmox-backup-proxy on localhost via https,
but by setting the environment variable `PROXMOX_DEBUG_API_CODE` to `1` the
tool directly calls the corresponding code.

.. WARNING:: Using `PROXMOX_DEBUG_API_CODE` can be dangerous and is only intended
   for debugging purposes. It is not intended for use on a production system.

