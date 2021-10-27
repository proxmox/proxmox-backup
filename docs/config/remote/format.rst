This file contains information used to access remote servers.

Each entry starts with the header ``remote: <name>``, followed by the
remote configuration options.

::

  remote: server1
	host server1.local
	auth-id sync@pbs
	...

  remote: ...


You can use the ``proxmox-backup-manager remote`` command to manipulate
this file.
