==========================
pmt
==========================

.. include:: ../epilog.rst

-------------------------------------------------------------
Control Linux Tape Devices
-------------------------------------------------------------

:Author: |AUTHOR|
:Version: Version |VERSION|
:Manual section: 1


Synopsis
========

.. include:: synopsis.rst


Common Options
==============

All command supports the following parameters to specify the tape device:

--device <path>  Path to the Linux tape device

--drive <name>  Use drive from Proxmox Backup Server configuration.


Commands generating output supports the ``--output-format``
parameter. It accepts the following values:

:``text``: Text format (default). Human readable.

:``json``: JSON (single line).

:``json-pretty``: JSON (multiple lines, nicely formatted).


Description
===========

.. include:: description.rst


ENVIRONMENT
===========

:TAPE: If set, replaces the `--device` option.

:PROXMOX_TAPE_DRIVE: If set, replaces the `--drive` option.


.. include:: ../pbs-copyright.rst
