Each entry starts with the header ``backup: <name>``, followed by the
job configuration options.

::

  backup: job1
	drive hh8
	pool p4
	store store3
	schedule daily

  backup: ...


You can use the ``proxmox-tape backup-job`` command to manipulate
this file.
