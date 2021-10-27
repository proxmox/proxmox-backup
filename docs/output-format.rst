Most commands that produce output support the ``--output-format``
parameter. This accepts the following values:

:``text``: Text format (default). Structured data is rendered as a table.

:``json``: JSON (single line).

:``json-pretty``: JSON (multiple lines, nicely formatted).


Also, the following environment variables can modify output behavior:

``PROXMOX_OUTPUT_FORMAT``
  Defines the default output format.

``PROXMOX_OUTPUT_NO_BORDER``
  If set (to any value), do not render table borders.

``PROXMOX_OUTPUT_NO_HEADER``
  If set (to any value), do not render table headers.

.. note:: The ``text`` format is designed to be human readable, and
   not meant to be parsed by automation tools. Please use the ``json``
   format if you need to process the output.
