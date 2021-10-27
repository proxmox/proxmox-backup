Each LTO drive configuration section starts with the header ``lto: <name>``,
followed by the drive configuration options.

Tape changer configurations start with the header ``changer: <name>``,
followed by the changer configuration options.

::

  lto: hh8
	changer sl3
	path /dev/tape/by-id/scsi-10WT065325-nst

  changer: sl3
	export-slots 14,15,16
	path /dev/tape/by-id/scsi-CJ0JBE0059


You can use the ``proxmox-tape drive`` and ``proxmox-tape changer``
commands to manipulate this file.

.. NOTE:: The ``virtual:`` drive type is experimental and should only be used
   for debugging.
