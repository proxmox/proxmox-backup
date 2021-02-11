Each entry starts with a header ``sync: <name>``, followed by the
job configuration options.

::

  sync: job1
	store store1
	remote-store store1
	remote lina

  sync: ...
	

You can use the ``proxmox-backup-manager sync-job`` command to manipulate
this file.
