Introduction
============

This documentationm is written in :term:`reStructuredText` and formatted with :term:`Sphinx`.

this is a test

Why Backup?
-----------

dfgfd fghfgh fh


Features
--------

:Proxmox VE: The `Proxmox Virtual Environment`_ is fully
   supported. You can backup :term:`virtual machine`\ s and
   :term:`container`\ s.

:GUI: We provide a graphical, web based user interface.

:Deduplication: Inkremental backup produces large amounts of duplicate
   data. The deduplication layer removes that redundancy and makes
   inkremental backup small and space efficient.

:Data Integrity: The built in `SHA-256`_ checksum algorithm assures the
   accuray and consistency of your backups.

:Remote Sync: It is possible to efficently synchronize data to remote
   sites. Only deltas containing new data are transfered.

:Performance: The whole software stack is written in :term:`Rust`,
   which provides high speed and memory efficiency.

:Open Source: No secrets. You have access to the whole source tree.

:Compression: Ultra fast `LZ4`_ compression is able to compress
   several gigabytes of data per second.

History
-------

history ...
