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


Backup Group
~~~~~~~~~~~~

We call the tuple ``<type>/<ID>`` a backup group. Such group
may contains one or more backup snapshots.


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
   automatically include mount points. Instead, you will see a short
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

Proxmox backup support client side encryption using AES-256 in GCM_
mode. You first need to create an encryption key in order to use that:

.. code-block:: console

  # proxmox-backup-client key create my-backup.key
  Encryption Key Password: **************

The key is password protected by default. If you do not need this
extra protection, you can also create it without a password:

.. code-block:: console

  # proxmox-backup-client key create /path/to/my-backup.key --kdf  none


.. code-block:: console

  # proxmox-backup-client backup etc.pxar:/etc --keyfile /path/to/my-backup.key
  Password: *********
  Encryption Key Password: **************
  ...


You can avoid having to enter the passwords by setting the environment
variables ``PBS_PASSWORD`` and ``PBS_ENCRYPTION_PASSWORD``.

.. todo:: Explain master-key


Restoring Data
~~~~~~~~~~~~~~

The regular creation of backups is a necessary step to avoid data
loss. More important, however, is the restoration. Be sure to perform
periodic recovery tests to ensure that you can access your data in
case of problems.

First, you need to find the snapshot you want to restore. The snapshot
command gives you a list of all snapshots on the server:

.. code-block:: console

  # proxmox-backup-client snapshots
  ...
  host/elsa/2019-12-03T09:30:15Z | 51788646825 | root.pxar catalog.pcat1 index.json
  host/elsa/2019-12-03T09:35:01Z | 51790622048 | root.pxar catalog.pcat1 index.json
  ...

You can also inspect the catalog to find specific files.

.. code-block:: console

  # proxmox-backup-client catalog dump host/elsa/2019-12-03T09:35:01Z
  ...
  d "./root.pxar.didx/etc/cifs-utils"
  l "./root.pxar.didx/etc/cifs-utils/idmap-plugin"
  d "./root.pxar.didx/etc/console-setup"
  ...

The restore command lets you restore a single archive from the
backup.

.. code-block:: console

  # proxmox-backup-client restore host/elsa/2019-12-03T09:35:01Z root.pxar /target/path/

You can instead simply download the contents of any archive using '-'
instead of ``/target/path``. This dumps the content to standard
output:

.. code-block:: console

  # proxmox-backup-client restore host/elsa/2019-12-03T09:35:01Z index.json -


Interactive Restores
^^^^^^^^^^^^^^^^^^^^

If you only want to restore a few individual files, it is often easier
to use the interactive recovery shell.

.. code-block:: console

  # proxmox-backup-client catalog shell host/elsa/2019-12-03T09:35:01Z root.pxar
  Starting interactive shell
  pxar:/ > ls
  bin        boot       dev        etc        home       lib        lib32
  ...

The interactive recovery shell is a minimalistic command line interface that
utilizes the metadata stored in the catalog for you to quickly list, navigate and
search files contained within a file archive.
You can select individual files as well as select files matched by a glob pattern
for restore.

The use of the catalog for navigation reduces the overhead otherwise caused by
network traffic and decryption, as instead of downloading and decrypting
individual encrypted chunks from the chunk-store to access the metadata, we only
need to download and decrypt the catalog.
The actual chunks are only accessed if the metadata in the catalog is not enough
or for the actual restore.

Similar to common UNIX shells ``cd`` and ``ls`` are the commands used to change
working directory and list directory contents of the archive.
``pwd`` shows the full path of the current working directory with respect to the
archive root.

Being able to quickly search the contents of the archive is a often needed feature.
That's where the catalog is most valuable.
For example:

.. code-block:: console

  pxar:/ > find etc/ **/*.txt --select
  "/etc/X11/rgb.txt"
  pxar:/ > list-selected
  etc/**/*.txt
  pxar:/ > restore-selected /target/path
  ...

This will find and print all files ending in ``.txt`` located in ``etc/`` or a
subdirectory and add the corresponding pattern to the list for subsequent restores.
``list-selected`` shows these patterns and ``restore-selected`` finally restores
all files in the archive matching the patterns to ``/target/path`` on the local
host. This will scan the whole archive.

With ``restore /target/path`` you can restore the sub-archive given by the current
working directory to the local target path ``/target/path`` on your host.
By additionally passing a glob pattern with ``--pattern <glob>``, the restore is
further limited to files matching the pattern.
For example:

.. code-block:: console

  pxar:/ > cd /etc/
  pxar:/etc/ > restore /target/ --pattern **/*.conf
  ...

The above will scan trough all the directories below ``/etc`` and restore all
files ending in ``.conf``.

.. todo:: Explain interactive restore in more detail

Mounting of Archives via FUSE
^^^^^^^^^^^^^^^^^^^^^^^^^^^^^

The :term:`FUSE` implementation for the pxar archive allows you to mount a
file archive as a read-only filesystem to a mountpoint on your host.

.. code-block:: console

  # proxmox-backup-client mount host/backup-client/2020-01-29T11:29:22Z root.pxar /mnt
  # ls /mnt
  bin   dev  home  lib32  libx32      media  opt   root  sbin  sys  usr
  boot  etc  lib   lib64  lost+found  mnt    proc  run   srv   tmp  var

This allows you to access the full content of the archive in a seamless manner.

.. note:: As the FUSE connection needs to fetch and decrypt chunks from the
    backup servers datastore, this can cause some additional network and CPU
    load on your host, depending on the operations you perform on the mounted
    filesystem.

To unmount the filesystem simply use the ``umount`` command on the mountpoint:

.. code-block:: console

  # umount /mnt

Login and Logout
~~~~~~~~~~~~~~~~

The client tool prompts you to enter the logon password as soon as you
want to access the backup server. The server checks your credentials
and responds with a ticket that is valid for two hours. The client
tool automatically stores that ticket and use it for further requests
to this server.

You can also manually trigger this login/logout using the login and
logout commands:

.. code-block:: console

  # proxmox-backup-client login
  Password: **********

To remove the ticket, simply issue a logout:

.. code-block:: console

  # proxmox-backup-client logout


Pruning and Removing Backups
~~~~~~~~~~~~~~~~~~~~~~~~~~~~

You can manually delete a backup snapshot using the ``forget``
command:

.. code-block:: console

  # proxmox-backup-client forget <snapshot>


.. caution:: This command removes all the archives in this backup
   snapshot so that they are inaccessible and unrecoverable.


Such manual removal is sometimes required, but normally the prune
command is used to systematically delete older backups. Prune lets
you specify which backup snapshots you want to keep. There are the
following retention options:

``--keep-last <N>``
  Keep the last ``<N>`` backup snapshots.

``--keep-hourly <N>``
  Keep backups for the last ``<N>`` different hours. If there is more than one
  backup for a single hour, only the latest one is kept.

``--keep-daily <N>``
  Keep backups for the last ``<N>`` different days. If there is more than one
  backup for a single day, only the latest one is kept.

``--keep-weekly <N>``
  Keep backups for the last ``<N>`` different weeks. If there is more than one
  backup for a single week, only the latest one is kept.

  .. note:: The weeks start on Monday and end on Sunday. The software
     uses the `ISO week date`_ system and correctly handles weeks at
     the end of the year.

``--keep-monthly <N>``
  Keep backups for the last ``<N>`` different months. If there is more than one
  backup for a single month, only the latest one is kept.

``--keep-yearly <N>``
  Keep backups for the last ``<N>`` different years. If there is more than one
  backup for a single year, only the latest one is kept.


Those retention options are processed in the order given above. Each
option covers a specific period of time. We say that backups within
this period are covered by this option. The next option does not take
care of already covered backups and only considers older backups.

The prune command also looks for unfinished and incomplete backups and
removes them unless they are newer than the last successful backup. In
this case, the last failed backup is retained.

.. code-block:: console

  # proxmox-backup-client prune <group> --keep-daily 7 --keep-weekly 4 --keep-monthly 3


You can use the ``--dry-run`` option to test your settings. This just
shows the list of existing snapshots and what action prune would take
on that.

.. code-block:: console

  # proxmox-backup-client prune host/elsa --dry-run --keep-daily 1 --keep-weekly 3
  retention options: --keep-daily 1 --keep-weekly 3
  Testing prune on store "store2" group "host/elsa"
  host/elsa/2019-12-04T13:20:37Z keep
  host/elsa/2019-12-03T09:35:01Z remove
  host/elsa/2019-11-22T11:54:47Z keep
  host/elsa/2019-11-21T12:36:25Z remove
  host/elsa/2019-11-10T10:42:20Z keep


.. note:: Neither the ``prune`` command nor the ``forget`` command free space
   in the chunk-store. The chunk-store still contains the data blocks
   unless you are performing :ref:`garbage-collection`.


.. _garbage-collection:

Garbage Collection
~~~~~~~~~~~~~~~~~~

The ``prune`` command removes only the backup index files, not the data
from the data store. This task is left to the garbage collection
command. It is therefore recommended to carry out garbage collection
regularly.

The garbage collection works in two phases. In the first phase, all
data blocks that are still in use are marked. In the second phase,
unused data blocks are removed.

.. note:: This command needs to read all existing backup index files
  and touches the complete chunk-store. This can take a long time
  depending on the number of chunks and the speed of the underlying
  disks.


.. code-block:: console

  # proxmox-backup-client garbage-collect
  starting garbage collection on store store2
  Start GC phase1 (mark used chunks)
  Start GC phase2 (sweep unused chunks)
  percentage done: 1, chunk count: 219
  percentage done: 2, chunk count: 453
  ...
  percentage done: 99, chunk count: 21188
  Removed bytes: 411368505
  Removed chunks: 203
  Original data bytes: 327160886391
  Disk bytes: 52767414743 (16 %)
  Disk chunks: 21221
  Average chunk size: 2486565
  TASK OK


.. todo:: howto run garbage-collection at regular intervalls (cron)


.. _pve-integration:

`Proxmox VE`_ integration
-------------------------


.. include:: command-line-tools.rst

.. include:: services.rst

.. include host system admin at the end

.. include:: sysadmin.rst
