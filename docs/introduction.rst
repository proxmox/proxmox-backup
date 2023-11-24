Introduction
============

What is Proxmox Backup Server?
------------------------------

`Proxmox Backup`_ Server is an enterprise-class, client-server backup solution
that is capable of backing up :term:`virtual machine<Virtual machine>`\ s,
:term:`container<Container>`\ s, and physical hosts. It is specially optimized
for the `Proxmox Virtual Environment`_ platform and allows you to back up your
data securely, even between remote sites, providing easy management through a
web-based user interface.

It supports deduplication, compression, and authenticated
encryption (AE_). Using :term:`Rust` as the implementation language guarantees
high performance, low resource usage, and a safe, high-quality codebase.

Proxmox Backup uses state of the art cryptography for both client-server
communication and backup content :ref:`encryption <client_encryption>`. All
client-server communication uses `TLS
<https://en.wikipedia.org/wiki/Transport_Layer_Security>`_, and backup data can
be encrypted on the client-side before sending, making it safer to back up data
to targets that are not fully trusted.

Architecture
------------

Proxmox Backup Server uses a `client-server model`_. The server stores the
backup data and provides an API to create and manage datastores. With the
API, it's also possible to manage disks and other server-side resources.

The backup client uses this API to access the backed up data. You can use the
``proxmox-backup-client`` command-line tool to create and restore file backups.
For QEMU_ and LXC_ within `Proxmox Virtual Environment`_, we deliver an
integrated client.

A single backup is allowed to contain several archives. For example, when you
backup a :term:`virtual machine<Virtual machine>`, each disk is stored as a
separate archive inside that backup. The VM configuration itself is stored as
an extra file. This way, it's easy to access and restore only the important
parts of the backup, without the need to scan the whole backup.


Main Features
-------------

:Support for Proxmox VE: The `Proxmox Virtual Environment`_ is fully supported,
   and you can easily backup :term:`virtual machine<Virtual machine>`\ s and
   :term:`container<Container>`\ s.

:Performance: The whole software stack is written in :term:`Rust`,
   in order to provide high speed and memory efficiency.

:Deduplication: Periodic backups produce large amounts of duplicate
   data. The deduplication layer avoids redundancy and minimizes the storage
   space used.

:Incremental backups: Changes between backups are typically low. Reading and
   sending only the delta reduces the storage and network impact of backups.

:Data integrity: The built-in `SHA-256`_ checksum algorithm ensures accuracy
   and consistency in your backups.

:Remote sync: It is possible to efficiently synchronize data to remote
   sites. Only deltas containing new data are transferred.

:Compression: The ultra-fast Zstandard_ compression is able to compress
   several gigabytes of data per second.

:Encryption: Backups can be encrypted on the client-side, using AES-256 GCM_.
   This authenticated encryption (AE_) mode provides very high performance on
   modern hardware. In addition to client-side encryption, all data is
   transferred via a secure TLS connection.

:Tape backup: For long-term archiving of data, Proxmox Backup Server also
   provides extensive support for backing up to tape and managing tape
   libraries.

:Ransomware protection: :ref:`Protect your critical data from ransomware attacks
   <ransomware_protection>` with Proxmox Backup Server's fine-grained access
   control, data integrity verification, and off-site backup through remote sync
   and tape backup.

:Web interface: Manage the Proxmox Backup Server with the integrated, web-based
   user interface.

:Open source: No secrets. Proxmox Backup Server is free and open-source
   software. The source code is licensed under AGPL, v3.

:No limits: Proxmox Backup Server has no artificial limits for backup storage or
   backup-clients.

:Enterprise support: Proxmox Server Solutions GmbH offers enterprise support in
   the form of `Proxmox Backup Server Subscription Plans
   <https://www.proxmox.com/en/proxmox-backup-server/pricing>`_. Users at every
   subscription level get access to the Proxmox Backup :ref:`Enterprise
   Repository <sysadmin_package_repos_enterprise>`. In addition, with a Basic,
   Standard or Premium subscription, users have access to the :ref:`Proxmox
   Customer Portal <get_help_enterprise_support>`.


Reasons for Data Backup?
------------------------

The main purpose of a backup is to protect against data loss. Data loss can be
caused by both faulty hardware and human error.

A common mistake is to accidentally delete a file or folder which is still
required. Virtualization can even amplify this problem, as deleting a whole
virtual machine can be as easy as pressing a single button.

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

Proxmox Backup Server consists of multiple components:

* A server-daemon providing, among other things, a RESTful API, super-fast
  asynchronous tasks, lightweight usage statistic collection, scheduling
  events, strict separation of privileged and unprivileged execution
  environments
* A JavaScript management web interface
* A management CLI tool for the server (`proxmox-backup-manager`)
* A client CLI tool (`proxmox-backup-client`) to access the server easily from
  any `Linux amd64` environment

Aside from the web interface, most parts of Proxmox Backup Server are written in
the Rust programming language.

 "The Rust programming language helps you write faster, more reliable software.
 High-level ergonomics and low-level control are often at odds in programming
 language design; Rust challenges that conflict. Through balancing powerful
 technical capacity and a great developer experience, Rust gives you the option
 to control low-level details (such as memory usage) without all the hassle
 traditionally associated with such control."

 -- `The Rust Programming Language <https://doc.rust-lang.org/book/ch00-00-introduction.html>`_

.. _get_help:

Getting Help
------------

.. _get_help_enterprise_support:

Enterprise Support
~~~~~~~~~~~~~~~~~~

Users with a `Proxmox Backup Server Basic, Standard or Premium Subscription Plan
<https://www.proxmox.com/en/proxmox-backup-server/pricing>`_ have access to the
`Proxmox Customer Portal <https://my.proxmox.com>`_. The customer portal
provides support with guaranteed response times from the Proxmox developers.
For more information or for volume discounts, please contact sales@proxmox.com.

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

:Mailing list for developers: `Proxmox Backup Server Development List`_

Bug Tracker
~~~~~~~~~~~

Proxmox runs a public bug tracker at `<https://bugzilla.proxmox.com>`_. If an
issue appears, file your report there. An issue can be a bug, as well as a
request for a new feature or enhancement. The bug tracker helps to keep track
of the issue and will send a notification once it has been solved.

License
-------

|pbs-copyright|

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

Backup is, and always has been, a central aspect of IT administration.
The need to recover from data loss is fundamental and only increases with
virtualization.

For this reason, we've been shipping a backup tool with Proxmox VE, from the
beginning. This tool is called ``vzdump`` and is able to make
consistent snapshots of running LXC containers and KVM virtual
machines.

However, ``vzdump`` only allows for full backups. While this is fine
for small backups, it becomes a burden for users with large VMs. Both
backup duration and storage usage are too high for this case, especially
for users who want to keep many backups of the same VMs. To solve these
problems, we needed to offer deduplication and incremental backups.

Back in October 2018, development started. We investigated
several technologies and frameworks and finally decided to use
:term:`Rust` as the implementation language, in order to provide high speed and
memory efficiency. The 2018-edition of Rust seemed promising for our
requirements.

In July 2020, we released the first beta version of Proxmox Backup
Server, followed by the first stable version in November 2020. With support for
encryption and incremental, fully deduplicated backups, Proxmox Backup offers a
secure environment, which significantly reduces network load and saves valuable
storage space.
