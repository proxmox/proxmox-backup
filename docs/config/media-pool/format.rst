Each entry starts with the header ``pool: <name>``, followed by the
media pool configuration options.

::

  pool: company1
	allocation always
	retention overwrite

  pool: ...


You can use the ``proxmox-tape pool`` command to manipulate this file.
