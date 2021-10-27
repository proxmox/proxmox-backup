Each entry starts with the header ``verification: <name>``, followed by the
job configuration options.

::

  verification: verify-store2
	ignore-verified true
	outdated-after 7
	schedule daily
	store store2

  verification: ...


You can use the ``proxmox-backup-manager verify-job`` command to manipulate
this file.
