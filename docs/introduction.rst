Introduction
============

What is Proxmox Backup Server
-----------------------------

Proxmox Backup Server is an enterprise-class client-server backup software that
backups :term:`virtual machine`\ s, :term:`container`\ s, and physical hosts.
It is specially optimized for the `Proxmox Virtual Environment`_ platform and
allows you to backup your data securely, even between remote sites, providing
easy management with a web-based user interface.

Proxmox Backup Server supports deduplication, compression, and authenticated
encryption (AE_). Using :term:`Rust` as implementation language guarantees high
performance, low resource usage, and a safe, high quality code base.

It features strong encryption done on the client side. Thus, it's possible to
backup data to not fully trusted targets.


Architecture
------------

Proxmox Backup Server uses a `client-server model`_. The server stores the
backup data and provides an API to create backups and restore data. With the
API it's also possible to manage disks and other server side resources.

The backup client uses this API to access the backed up data. With the command
line tool ``proxmox-backup-client`` you can create backups and restore data.
For QEMU_ with `Proxmox Virtual Environment`_ we deliver an integrated client.

A single backup is allowed to contain several archives. For example, when you
backup a :term:`virtual machine`, each disk is stored as a separate archive
inside that backup. The VM configuration itself is stored as an extra file.
This way, it is easy to access and restore only important parts of the backup
without the need to scan the whole backup.


Main Features
-------------

:Support for Proxmox VE: The `Proxmox Virtual Environment`_ is fully
   supported and you can easily backup :term:`virtual machine`\ s and
   :term:`container`\ s.

:Performance: The whole software stack is written in :term:`Rust`,
   to provide high speed and memory efficiency.

:Deduplication: Periodic backups produce large amounts of duplicate
   data. The deduplication layer avoids redundancy and minimizes the used
   storage space.

:Incremental backups: Changes between backups are typically low. Reading and
   sending only the delta reduces storage and network impact of backups.

:Data Integrity: The built-in `SHA-256`_ checksum algorithm assures the
   accuracy and consistency of your backups.

:Remote Sync: It is possible to efficiently synchronize data to remote
   sites. Only deltas containing new data are transferred.

:Compression: The ultra fast Zstandard_ compression is able to compress
   several gigabytes of data per second.

:Encryption: Backups can be encrypted on the client-side using AES-256 in
   Galois/Counter Mode (GCM_) mode. This authenticated encryption (AE_) mde
   provides very high performance on modern hardware.

:Web interface: Manage the Proxmox Backup Server with the integrated web-based
   user interface.

:Open Source: No secrets. Proxmox Backup Server is free and open-source
   software. The source code is licensed under AGPL, v3.

:Support: Enterprise support will be available from `Proxmox`_ once the beta
   phase is over.


Reasons for Data Backup?
------------------------

The main purpose of a backup is to protect against data loss. Data loss can be
caused by faulty hardware but also by human error.

A common mistake is to accidentally delete a file or folder which is still
required. Virtualization can even amplify this problem; it easily happens that
a whole virtual machine is deleted by just pressing a single button.

For administrators, backups can serve as a useful toolkit for temporarily
storing data. For example, it is common practice to perform full backups before
installing major software updates. If something goes wrong, you can easily
restore the previous state.

Another reason for backups are legal requirements. Some data, especially
business records, must be kept in a safe place for several years by law, so
that they can be accessed if required.

In general, data loss is very costly as it can severely damage your business.
Therefore, ensure that you perform regular backups and run restore tests.


Software Stack
--------------

.. todo:: Eplain why we use Rust (and Flutter)
	  

Getting Help
------------

Community Support Forum
~~~~~~~~~~~~~~~~~~~~~~~

We always encourage our users to discuss and share their knowledge using the
`Proxmox Community Forum`_. The forum is moderated by the Proxmox support team.
The large user base is spread out all over the world. Needless to say that such
a large forum is a great place to get information.

Mailing Lists
~~~~~~~~~~~~~

Proxmox Backup Server is fully open-source and contributions are welcome! Here
is the primary communication channel for developers:
:Mailing list for developers: `PBS Development List`_

Bug Tracker
~~~~~~~~~~~

Proxmox runs a public bug tracker at `<https://bugzilla.proxmox.com>`_. If an
issue appears, file your report there. An issue can be a bug as well as a
request for a new feature or enhancement. The bug tracker helps to keep track
of the issue and will send a notification once it has been solved.

License
-------

Copyright (C) 2019-2020 Proxmox Server Solutions GmbH

This software is written by Proxmox Server Solutions GmbH <support@proxmox.com>

Proxmox Backup Server is free and open source software: you can use it,
redistribute it, and/or modify it under the terms of the GNU Affero General
Public License as published by the Free Software Foundation, either version 3
of the License, or (at your option) any later version.

This program is distributed in the hope that it will be useful, but
``WITHOUT ANY WARRANTY``; without even the implied warranty of
``MERCHANTABILITY`` or ``FITNESS FOR A PARTICULAR PURPOSE``.  See the GNU
Affero General Public License for more details.

You should have received a copy of the GNU Affero General Public License
along with this program.  If not, see AGPL3_.


History
-------

.. todo:: Add development History of the product

