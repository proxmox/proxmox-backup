Backup Storage
==============

.. _storage_disk_management:

Disk Management
---------------

.. image:: images/screenshots/pbs-gui-disks.png
  :align: right
  :alt: List of disks

Proxmox Backup Server comes with a set of disk utilities, which are
accessed using the ``disk`` subcommand or the web interface. This subcommand
allows you to initialize disks, create various filesystems, and get information
about the disks.

.. image:: images/screenshots/pbs-gui-disks.png
  :align: right
  :alt: Web Interface Administration: Disks

To view the disks connected to the system, navigate to **Administration ->
Storage/Disks** in the web interface or use the ``list`` subcommand of
``disk``:

.. code-block:: console

  # proxmox-backup-manager disk list
  ┌──────┬────────┬─────┬───────────┬─────────────┬───────────────┬─────────┬────────┐
  │ name │ used   │ gpt │ disk-type │        size │ model         │ wearout │ status │
  ╞══════╪════════╪═════╪═══════════╪═════════════╪═══════════════╪═════════╪════════╡
  │ sda  │ lvm    │   1 │ hdd       │ 34359738368 │ QEMU_HARDDISK │       - │ passed │
  ├──────┼────────┼─────┼───────────┼─────────────┼───────────────┼─────────┼────────┤
  │ sdb  │ unused │   1 │ hdd       │ 68719476736 │ QEMU_HARDDISK │       - │ passed │
  ├──────┼────────┼─────┼───────────┼─────────────┼───────────────┼─────────┼────────┤
  │ sdc  │ unused │   1 │ hdd       │ 68719476736 │ QEMU_HARDDISK │       - │ passed │
  └──────┴────────┴─────┴───────────┴─────────────┴───────────────┴─────────┴────────┘

To initialize a disk with a new GPT, use the ``initialize`` subcommand:

.. code-block:: console

  # proxmox-backup-manager disk initialize sdX

.. image:: images/screenshots/pbs-gui-disks-dir-create.png
  :align: right
  :alt: Create a directory

You can create an ``ext4`` or ``xfs`` filesystem on a disk using ``fs
create``, or by navigating to **Administration -> Storage/Disks -> Directory**
in the web interface and creating one from there. The following command creates
an ``ext4`` filesystem and passes the ``--add-datastore`` parameter, in order to
automatically create a datastore on the disk (in this case ``sdd``). This will
create a datastore at the location ``/mnt/datastore/store1``:

.. code-block:: console

  # proxmox-backup-manager disk fs create store1 --disk sdd --filesystem ext4 --add-datastore true

.. image:: images/screenshots/pbs-gui-disks-zfs-create.png
  :align: right
  :alt: Create ZFS

You can also create a ``zpool`` with various raid levels from **Administration
-> Storage/Disks -> ZFS** in the web interface, or by using ``zpool create``. The command
below creates a mirrored ``zpool`` using two disks (``sdb`` & ``sdc``) and
mounts it under ``/mnt/datastore/zpool1``:

.. code-block:: console

  # proxmox-backup-manager disk zpool create zpool1 --devices sdb,sdc --raidlevel mirror

.. note:: You can also pass the ``--add-datastore`` parameter here, to automatically
  create a datastore from the disk.

You can use ``disk fs list`` and ``disk zpool list`` to keep track of your
filesystems and zpools respectively.

Proxmox Backup Server uses the package smartmontools. This is a set of tools
used to monitor and control the S.M.A.R.T. system for local hard disks. If a
disk supports S.M.A.R.T. capability, and you have this enabled, you can
display S.M.A.R.T. attributes from the web interface or by using the command:

.. code-block:: console

  # proxmox-backup-manager disk smart-attributes sdX

.. note:: This functionality may also be accessed directly through the use of
  the ``smartctl`` command, which comes as part of the smartmontools package
  (see ``man smartctl`` for more details).


.. _datastore_intro:

:term:`Datastore`
-----------------

.. image:: images/screenshots/pbs-gui-datastore-summary.png
  :align: right
  :alt: Datastore Usage Overview

A datastore refers to a location at which backups are stored. The current
implementation uses a directory inside a standard Unix file system (``ext4``,
``xfs`` or ``zfs``) to store the backup data.

Datastores are identified by a simple *ID*. You can configure this
when setting up the datastore. The configuration information for datastores
is stored in the file ``/etc/proxmox-backup/datastore.cfg``.

.. note:: The `File Layout`_ requires the file system to support at least *65538*
   subdirectories per directory. That number comes from the 2\ :sup:`16`
   pre-created chunk namespace directories, and the ``.`` and ``..`` default
   directory entries. This requirement excludes certain filesystems and
   filesystem configurations from being supported for a datastore. For example,
   ``ext3`` as a whole or ``ext4`` with the ``dir_nlink`` feature manually disabled.


Datastore Configuration
~~~~~~~~~~~~~~~~~~~~~~~

.. image:: images/screenshots/pbs-gui-datastore-content.png
  :align: right
  :alt: Datastore Content Overview

You can configure multiple datastores. A minimum of one datastore needs to be
configured. The datastore is identified by a simple *name* and points to a
directory on the filesystem. Each datastore also has associated retention
settings of how many backup snapshots for each interval of ``hourly``,
``daily``, ``weekly``, ``monthly``, ``yearly`` as well as a time-independent
number of backups to keep in that store. :ref:`backup-pruning` and
:ref:`garbage collection <client_garbage-collection>` can also be configured to
run periodically, based on a configured schedule (see
:ref:`calendar-event-scheduling`) per datastore.


.. _storage_datastore_create:

Creating a Datastore
^^^^^^^^^^^^^^^^^^^^
.. image:: images/screenshots/pbs-gui-datastore-create.png
  :align: right
  :alt: Create a datastore

You can create a new datastore from the web interface, by clicking **Add
Datastore** in the side menu, under the **Datastore** section. In the setup
window:

* *Name* refers to the name of the datastore
* *Backing Path* is the path to the directory upon which you want to create the
  datastore
* *GC Schedule* refers to the time and intervals at which garbage collection
  runs
* *Prune Schedule* refers to the frequency at which pruning takes place
* *Prune Options* set the amount of backups which you would like to keep (see
  :ref:`backup-pruning`).
* *Comment* can be used to add some contextual information to the datastore.

Alternatively you can create a new datastore from the command line. The
following command creates a new datastore called ``store1`` on
:file:`/backup/disk1/store1`

.. code-block:: console

  # proxmox-backup-manager datastore create store1 /backup/disk1/store1


Managing Datastores
^^^^^^^^^^^^^^^^^^^

To list existing datastores from the command line, run:

.. code-block:: console

  # proxmox-backup-manager datastore list
  ┌────────┬──────────────────────┬─────────────────────────────┐
  │ name   │ path                 │ comment                     │
  ╞════════╪══════════════════════╪═════════════════════════════╡
  │ store1 │ /backup/disk1/store1 │ This is my default storage. │
  └────────┴──────────────────────┴─────────────────────────────┘

You can change the garbage collection and prune settings of a datastore, by
editing the datastore from the GUI or by using the ``update`` subcommand. For
example, the below command changes the garbage collection schedule using the
``update`` subcommand and prints the properties of the datastore with the
``show`` subcommand:

.. code-block:: console

  # proxmox-backup-manager datastore update store1 --gc-schedule 'Tue 04:27'
  # proxmox-backup-manager datastore show store1
  ┌────────────────┬─────────────────────────────┐
  │ Name           │ Value                       │
  ╞════════════════╪═════════════════════════════╡
  │ name           │ store1                      │
  ├────────────────┼─────────────────────────────┤
  │ path           │ /backup/disk1/store1        │
  ├────────────────┼─────────────────────────────┤
  │ comment        │ This is my default storage. │
  ├────────────────┼─────────────────────────────┤
  │ gc-schedule    │ Tue 04:27                   │
  ├────────────────┼─────────────────────────────┤
  │ keep-last      │ 7                           │
  ├────────────────┼─────────────────────────────┤
  │ prune-schedule │ daily                       │
  └────────────────┴─────────────────────────────┘

Finally, it is possible to remove the datastore configuration:

.. code-block:: console

  # proxmox-backup-manager datastore remove store1

.. note:: The above command removes only the datastore configuration. It does
   not delete any data from the underlying directory.


File Layout
^^^^^^^^^^^

After creating a datastore, the following default layout will appear:

.. code-block:: console

  # ls -arilh /backup/disk1/store1
  276493 -rw-r--r-- 1 backup backup       0 Jul  8 12:35 .lock
  276490 drwxr-x--- 1 backup backup 1064960 Jul  8 12:35 .chunks

`.lock` is an empty file used for process locking.

The `.chunks` directory contains folders, starting from `0000` and increasing in
hexadecimal values until `ffff`. These directories will store the chunked data,
categorized by checksum, after a backup operation has been executed.

.. code-block:: console

 # ls -arilh /backup/disk1/store1/.chunks
 545824 drwxr-x--- 2 backup backup 4.0K Jul  8 12:35 ffff
 545823 drwxr-x--- 2 backup backup 4.0K Jul  8 12:35 fffe
 415621 drwxr-x--- 2 backup backup 4.0K Jul  8 12:35 fffd
 415620 drwxr-x--- 2 backup backup 4.0K Jul  8 12:35 fffc
 353187 drwxr-x--- 2 backup backup 4.0K Jul  8 12:35 fffb
 344995 drwxr-x--- 2 backup backup 4.0K Jul  8 12:35 fffa
 144079 drwxr-x--- 2 backup backup 4.0K Jul  8 12:35 fff9
 144078 drwxr-x--- 2 backup backup 4.0K Jul  8 12:35 fff8
 144077 drwxr-x--- 2 backup backup 4.0K Jul  8 12:35 fff7
 ...
 403180 drwxr-x--- 2 backup backup 4.0K Jul  8 12:35 000c
 403179 drwxr-x--- 2 backup backup 4.0K Jul  8 12:35 000b
 403177 drwxr-x--- 2 backup backup 4.0K Jul  8 12:35 000a
 402530 drwxr-x--- 2 backup backup 4.0K Jul  8 12:35 0009
 402513 drwxr-x--- 2 backup backup 4.0K Jul  8 12:35 0008
 402509 drwxr-x--- 2 backup backup 4.0K Jul  8 12:35 0007
 276509 drwxr-x--- 2 backup backup 4.0K Jul  8 12:35 0006
 276508 drwxr-x--- 2 backup backup 4.0K Jul  8 12:35 0005
 276507 drwxr-x--- 2 backup backup 4.0K Jul  8 12:35 0004
 276501 drwxr-x--- 2 backup backup 4.0K Jul  8 12:35 0003
 276499 drwxr-x--- 2 backup backup 4.0K Jul  8 12:35 0002
 276498 drwxr-x--- 2 backup backup 4.0K Jul  8 12:35 0001
 276494 drwxr-x--- 2 backup backup 4.0K Jul  8 12:35 0000
 276489 drwxr-xr-x 3 backup backup 4.0K Jul  8 12:35 ..
 276490 drwxr-x--- 1 backup backup 1.1M Jul  8 12:35 .


Once you uploaded some backups, or created namespaces, you may see the Backup
Type (`ct`, `vm`, `host`) and the start of the namespace hierachy (`ns`).

.. _storage_namespaces:

Backup Namespaces
~~~~~~~~~~~~~~~~~

A datastore can host many backups as long as the underlying storage is big
enough and provides the performance required for one's use case.
But, without any hierarchy or separation its easy to run into naming conflicts,
especially when using the same datastore for multiple Proxmox VE instances or
multiple users.

The backup namespace hierarchy allows you to clearly separate different users
or backup sources in general, avoiding naming conflicts and providing
well-organized backup content view.

Each namespace level can host any backup type, CT, VM or Host but also other
namespaces, up to a depth of 8 level, where the root namespace is the first
level.


Namespace Permissions
^^^^^^^^^^^^^^^^^^^^^

You can make the permission configuration of a datastore more fine-grained by
setting permissions only on a specific namespace.

To see a datastore you need permission that has at least one of `AUDIT`,
`MODIFY`, `READ` or `BACKUP` privilege on any namespace it contains.

To create or delete a namespace you require the modify privilege on the parent
namespace. So, to initially create namespaces you need to have a permission
with a access role that includes the `MODIFY` privilege on the datastore itself.

For backup groups the existing privilege rules still apply, you either need a
powerful permission or be the owner of the backup group, nothing changed here.

.. todo:: continue


Options
~~~~~~~

.. image:: images/screenshots/pbs-gui-datastore-options.png
  :align: right
  :alt: Datastore Options

There are a few per-datastore options:

* :ref:`Notifications <maintenance_notification>`
* :ref:`Maintenance Mode <maintenance_mode>`
* Verification of incoming backups
