.. _terms:

Terminology
===========

Backup Content
--------------

When doing deduplication, there are different strategies to get
optimal results in terms of performance and/or deduplication rates.
Depending on the type of data, it can be split into *fixed* or *variable*
sized chunks.

Fixed sized chunking requires minimal CPU power, and is used to
backup virtual machine images.

Variable sized chunking needs more CPU power, but is essential to get
good deduplication rates for file archives.

The `Proxmox Backup`_ Server supports both strategies.


Image Archives: ``<name>.img``
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

This is used for virtual machine images and other large binary
data. Content is split into fixed-sized chunks.


File Archives: ``<name>.pxar``
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

.. see https://moinakg.wordpress.com/2013/06/22/high-performance-content-defined-chunking/

A file archive stores a full directory tree. Content is stored using
the :ref:`pxar-format`, split into variable-sized chunks. The format
is optimized to achieve good deduplication rates.


Binary Data (BLOBs)
~~~~~~~~~~~~~~~~~~~

This type is used to store smaller (< 16MB) binary data such as
configuration files. Larger files should be stored as image archives.

.. caution:: Please do not store all files as BLOBs. Instead, use the
   file archive to store entire directory trees.


Catalog File: ``catalog.pcat1``
~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~

The catalog file is an index for file archives. It contains
the list of included files and is used to speed up search operations.


The Manifest: ``index.json``
~~~~~~~~~~~~~~~~~~~~~~~~~~~~

The manifest contains a list of all backed up files, and their
sizes and checksums. It is used to verify the consistency of a
backup.

Backup Namespace
----------------

Namespaces allow for the reuse of a single chunk store deduplication domain for
multiple sources, while avoiding naming conflicts and enabling more fine-grained
access control.

Essentially, they're implemented as a simple directory structure and don't
require separate configuration.

Backup Type
-----------

The backup server groups backups by *type*, where *type* is one of:

``vm``
    This type is used for :term:`virtual machine<Virtual machine>`\ s. It
    typically consists of the virtual machine's configuration file and an image
    archive for each disk.

``ct``
    This type is used for :term:`container<Container>`\ s. It consists of the
    container's configuration and a single file archive for the filesystem's
    contents.

``host``
    This type is used for file/directory backups created from within a machine.
    Typically this would be a physical host, but could also be a virtual machine
    or container. Such backups may contain file and image archives; there are no
    restrictions in this regard.

Backup ID
---------

A unique ID for a specific Backup Type and Backup Namespace. Usually the
virtual machine or container ID. ``host`` type backups normally use the
hostname.

Backup Time
-----------

The time when the backup was made with second resolution.


Backup Group
------------

The tuple ``<type>/<id>`` is called a backup group. Such a group may contain
one or more backup snapshots.


.. _term_backup_snapshot:

Backup Snapshot
---------------

The triplet ``<type>/<ID>/<time>`` is called a backup snapshot. It
uniquely identifies a specific backup within a namespace.

.. code-block:: console
   :caption: Backup Snapshot Examples

    vm/104/2019-10-09T08:01:06Z
    host/elsa/2019-11-08T09:48:14Z

As you can see, the time format is RFC3339_ with Coordinated
Universal Time (UTC_, identified by the trailing *Z*).


