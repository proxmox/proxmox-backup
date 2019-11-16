Introduction
============

This documentationm is written in :term:`reStructuredText` and formatted with :term:`Sphinx`.


What is Proxmox Backup
----------------------

Proxmox Backup is an enterprise class client-server backup software,
specially optimized for `Proxmox Virtual Environment`_ to backup
:term:`virtual machine`\ s and :term:`container`\ s. It is also
possible to backup physical hosts.

It supports deduplication, compression and authenticated encryption
(AE_). Using RUST_ as implementation language guarantees high
performance, low resource usage, and a safe, high quality code base.

Encryption is done at the client side. This makes backups to not fully
trusted targets possible.


Architecture
------------

Proxmox Backup uses a `Client-server model`_. The server is
responsible to store the backup data, and provides an API to create
backups and restore data. It is also possible to manage disks and
other server side resources using this API.

A backup client uses this API to access the backed up data,
i.e. ``proxmox-backup-client`` is a command line tool to create
backups and restore data. We also deliver an integrated client for
QEMU_ with `Proxmox Virtual Environment`_.


Main features
-------------

:Proxmox VE: The `Proxmox Virtual Environment`_ is fully
   supported. You can backup :term:`virtual machine`\ s and
   :term:`container`\ s.

:GUI: We provide a graphical, web based user interface.

:Deduplication: Incremental backup produces large amounts of duplicate
   data. The deduplication layer removes that redundancy and makes
   inkremental backup small and space efficient.

:Data Integrity: The built in `SHA-256`_ checksum algorithm assures the
   accuray and consistency of your backups.

:Remote Sync: It is possible to efficently synchronize data to remote
   sites. Only deltas containing new data are transfered.

:Performance: The whole software stack is written in :term:`Rust`,
   which provides high speed and memory efficiency.

:Compression: Ultra fast Zstandard_ compression is able to compress
   several gigabytes of data per second.

:Encryption: Backups can be encrypted at client side using AES-256 in
   GCM_ mode. This authenticated encryption mode (AE_) provides very
   high performance on modern hardware.

:Open Source: No secrets. You have access to the whole source tree.

:Support: Commercial support options available from `Proxmox`_.


Why Backup?
-----------

The primary purpose of backup is to protect against data loss. Data
loss can happen because of faulty hardware, but also by human errors.

A common mistake is to delete a file or folder which is still
required. Virtualization can amplify this problem, because it is now
easy to delete a whole virtual machine by a single button press.

Backups can also serve as a toolkit for administrators to temporarily
store data. For example, it is common practice to perform full backups
before installing major software updates. If something goes wrong, you
can just restore the previous state.

Another reason for backups are legal requirements. Some data must be
kept in a safe place for several years so that you can access it if
required by law.


- value of data, importance for your business, legal regulations

- restore tests. to make sure backup/restore works


Software Stack
--------------


License
-------

Copyright (C) 2019 Proxmox Server Solutions GmbH

This software is written by Proxmox Server Solutions GmbH <support@proxmox.com>

Proxmox Backup is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as
published by the Free Software Foundation, either version 3 of the
License, or (at your option) any later version.

This program is distributed in the hope that it will be useful, but
``WITHOUT ANY WARRANTY``; without even the implied warranty of
``MERCHANTABILITY`` or ``FITNESS FOR A PARTICULAR PURPOSE``.  See the GNU
Affero General Public License for more details.

You should have received a copy of the GNU Affero General Public License
along with this program.  If not, see AGPL3_.


History
-------

history ...
