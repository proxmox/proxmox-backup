All commands support the following parameters to specify the tape device:

--device <path>  Path to the Linux tape device

--drive <name>  Use drive from Proxmox Backup Server configuration.


Commands which generate output support the ``--output-format``
parameter. It accepts the following values:

:``text``: Text format (default). Human readable.

:``json``: JSON (single line).

:``json-pretty``: JSON (multiple lines, nicely formatted).


Device driver options can be specified as integer numbers (see
``/usr/include/linux/mtio.h``), or using symbolic names:

:``buffer-writes``: Enable buffered writes

:``async-writes``:  Enable async writes

:``read-ahead``: Use read-ahead for fixed block size

:``debugging``: Enable debugging if compiled into the driver

:``two-fm``:  Write two file marks when closing the file

:``fast-mteom``: Space directly to eod (and lose file number)

:``auto-lock``: Automatically lock/unlock drive door

:``def-writes``: Defaults are meant only for writes

:``can-bsr``: Indicates that the drive can space backwards

:``no-blklims``: Drive does not support read block limits

:``can-partitions``: Drive can handle partitioned tapes

:``scsi2locical``: Seek and tell use SCSI-2 logical block addresses

:``sysv``: Enable the System V semantics

:``nowait``:  Do not wait for rewind, etc. to complete

:``sili``: Enables setting the SILI bit in SCSI commands when reading
   in variable block mode to enhance performance when reading blocks
   shorter than the byte count
