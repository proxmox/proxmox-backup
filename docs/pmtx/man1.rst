==========================
pmtx
==========================

Synopsis
==========

.. include:: synopsis.rst

Common Options
==============

All command supports the following parameters to specify the changer device:

--device <path>  Path to Linux generic SCSI device (e.g. '/dev/sg4')

--changer <name>  Use changer from Proxmox Backup Server configuration.


Commands generating output supports the ``--output-format``
parameter. It accepts the following values:

:``text``: Text format (default). Human readable.

:``json``: JSON (single line).

:``json-pretty``: JSON (multiple lines, nicely formatted).

Description
============

.. include:: description.rst

ENVIRONMENT
===========

:CHANGER: If set, replaces the `--device` option

:PROXMOX_TAPE_DRIVE: If set, use the Proxmox Backup Server
   configuration to find the associated changer device.

.. include:: ../pbs-copyright.rst
