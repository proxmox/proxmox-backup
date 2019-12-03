Administration Guide
====================

The administration guide.


Terminology
-----------

Backup Content
~~~~~~~~~~~~~~

When doing deduplication, there are different strategies to get
optimal results in terms of performance and/or deduplication rates.
Depending on the type of data, one can split data into fixed or variable
sized chunks.

Fixed sized chunking needs almost no CPU performance, and is used to
backup virtual machine images.

Variable sized chunking needs more CPU power, but is essential to get
good deduplication rates for file archives.

Therefore, the backup server supports both strategies.


File Archives: ``<name>.pxar``
^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^

.. see https://moinakg.wordpress.com/2013/06/22/high-performance-content-defined-chunking/

A file archive stores a whole directory tree. Content is stored using
the :ref:`pxar-format`, split into variable sized chunks. The format
is specially optimized to achieve good deduplication rates.


Image Archives: ``<name>.img``
^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^

This is used for virtual machine images and other large binary
data. Content is split into fixed sized chunks.


Binary Data (BLOBs)
^^^^^^^^^^^^^^^^^^^

This type is used to store smaller (< 16MB) binaries like
configuration data. Larger files should be stored as image archive.

.. caution:: Please do not store all files as BLOBs. Instead, use the
   file archive to store whole directory trees.


Catalog File: ``catalog.pcat1``
^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^

The catalog file is basically an index for file archive. It contains
the list of files, and is used to speedup search operations.


The Manifest: ``index.json``
^^^^^^^^^^^^^^^^^^^^^^^^^^^^

The manifest contains the list of all backup files, including
file sizes and checksums. It is used to verify the consistency of a
backup.


Backup Type
~~~~~~~~~~~

The backup server groups backups by *type*, where *type* is one of:

``vm``
    This type is used for :term:`virtual machine`\ s. Typically
    contains the virtual machine configuration and an image archive
    for each disk.

``ct``
    This type is used for :term:`container`\ s. Contains the container
    configuration and a single file archive for the container content.

``host``
    This type is used for physical host, or if you want to run backups
    manually from inside virtual machines or containers. Such backups
    may contain file and image archives (no restrictions here).


Backup ID
~~~~~~~~~

An unique ID. Usually the virtual machine or container ID. ``host``
type backups normally use the hostname.


Backup Time
~~~~~~~~~~~

The time when the backup was made.


Backup Snapshot
~~~~~~~~~~~~~~~

We call the triplet ``<type>/<ID>/<time>`` a backup snapshot. It
uniquely identifies a specific backup within a datastore.

.. code-block:: console
   :caption: Backup Snapshot Examples

    vm/104/2019-10-09T08:01:06Z
    host/elsa/2019-11-08T09:48:14Z

As you can see, the time is formatted as RFC3399_ using Coordinated
Universal Time (UTC_, identified by the trailing *Z*).


:term:`DataStore`
~~~~~~~~~~~~~~~~~

A datastore is a place to store backups. The current implementation
uses a directory inside a standard unix file system (``ext4``, ``xfs``
or ``zfs``) to store backup data.

Datastores are identified by a simple *ID*. You can configure that
when setting up the backup server.


Backup Server Management
------------------------

The command line tool to configure and manage the server is called
:command:`proxmox-backup-manager`.


Datastore Configuration
~~~~~~~~~~~~~~~~~~~~~~~

A :term:`datastore` is a place to store backups. You can configure
several datastores, but you need at least one of them. The datastore is identified by a simple `name` and point to a directory.

The following command creates a new datastore called ``store1`` on :file:`/backup/disk1/store1`

.. code-block:: console

  # proxmox-backup-manager datastore create store1 /backup/disk1/store1

To list existing datastores use:

.. code-block:: console

  # proxmox-backup-manager datastore list
  store1 /backup/disk1/store1

Finally, it is also possible to remove the datastore configuration:

.. code-block:: console

  # proxmox-backup-manager datastore remove store1

.. note:: Above command removes the datastore configuration. It does
   not delete any data from the underlying directory.


File Layout
^^^^^^^^^^^

.. todo:: Add datastore file layout example


Backup Client usage
-------------------

The command line client is called :command:`proxmox-backup-client`.


Respository Locations
~~~~~~~~~~~~~~~~~~~~~

The client uses a special repository notation to specify a datastore
on the backup server.

  [[username@]server:]datastore

If you do not specify a ``username`` the default is ``root@pam``. The
default for server is to use the local host (``localhost``).

You can pass the repository by setting the ``--repository`` command
line options, or by setting the ``PBS_REPOSITORY`` environment
variable.


Environment Variables
~~~~~~~~~~~~~~~~~~~~~~

``PBS_REPOSITORY``
  The default backup repository.

``PBS_PASSWORD``
  When set, this value is used for the password required for the
  backup server.

``PBS_ENCRYPTION_PASSWORD``

  When set, this value is used to access the secret encryption key (if
  protected by password).


Creating Backups
~~~~~~~~~~~~~~~~

This section explains how to create backup on physical host, or from
inside virtual machines or containers. Such backups may contain file
and image archives (no restrictions here).

.. note:: If you want to backup virtual machines or containers see :ref:`pve-integration`.

The prerequisite is that you have already set up (or can access) a
backup server. It is assumed that you know the repository name and
credentials. In the following examples we simply use ``backup-server:store1``.

.. code-block:: console

  # proxmox-backup-client backup root.pxar:/ --repository backup-server:store1
  Starting backup: host/elsa/2019-12-03T09:35:01Z
  Client name: elsa
  skip mount point: "/boot/efi"
  skip mount point: "/dev"
  skip mount point: "/run"
  skip mount point: "/sys"
  Uploaded 12129 chunks in 87 seconds (564 MB/s).
  End Time: 2019-12-03T10:36:29+01:00

This will prompt you for a password and then uploads a file archive named
``root.pxar`` containing all the files in the ``/`` directory.

.. Caution:: Please note that proxmox-backup-client does not
   automatically include mount points. Insted, you will see a short
   ``skip mount point`` notice for each of them. The idea is that you
   create a separate file archive for each mounted disk. You can also
   explicitly include them using the ``--include-dev`` option
   (i.e. ``--include-dev /boot/efi``). You can use this option
   multiple times, once for each mount point you want to include.

The ``--repository`` option is sometimes quite long and is used by all
commands. You can avoid having to enter this value by setting the
environment variable ``PBS_REPOSITORY``.

.. code-block:: console

  # export PBS_REPOSTORY=backup-server:store1

You can then execute all commands without specifying the ``--repository``
option.

One signle backup is allowed to contain more than one archive. For example, assume you want to backup two disks mounted at ``/mmt/disk1`` and ``/mnt/disk2``:

.. code-block:: console

  # proxmox-backup-client backup disk1.pxar:/mnt/disk1 disk2.pxar:/mnt/disk2

This create a backup of both disks.

The backup command takes a list of backup specifications, which
include archive name on the server, the type of the archive, and the
archive source at the client. The format is quite simple to understand:

    <archive-name>.<type>:<source-path>

Common types are ``.pxar`` for file archives, and ``.img`` for block
device images. Thus it is quite easy to create a backup for a block
device:

.. code-block:: console

  # proxmox-backup-client backup mydata.img:/dev/mylvm/mydata




Encryption
^^^^^^^^^^


Restoring Data
~~~~~~~~~~~~~~


.. _pve-integration:

`Proxmox VE`_ integration
-------------------------


.. include:: command-line-tools.rst

.. include:: services.rst

.. include host system admin at the end

.. include:: sysadmin.rst
