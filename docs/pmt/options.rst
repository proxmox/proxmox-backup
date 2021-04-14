All commands support the following parameters to specify the tape device:

--device <path>  Path to the Linux tape device

--drive <name>  Use drive from Proxmox Backup Server configuration.


Commands which generate output support the ``--output-format``
parameter. It accepts the following values:

:``text``: Text format (default). Human readable.

:``json``: JSON (single line).

:``json-pretty``: JSON (multiple lines, nicely formatted).
