Backup Management
=================

.. The administration guide.
 .. todo:: either add a bit more explanation or remove the previous sentence

Terminology
-----------

Backup Content
~~~~~~~~~~~~~~

When doing deduplication, there are different strategies to get
optimal results in terms of performance and/or deduplication rates.
Depending on the type of data, it can be split into *fixed* or *variable*
sized chunks.

Fixed sized chunking requires minimal CPU power, and is used to
backup virtual machine images.

Variable sized chunking needs more CPU power, but is essential to get
good deduplication rates for file archives.

The Proxmox Backup Server supports both strategies.


File Archives: ``<name>.pxar``
^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^

.. see https://moinakg.wordpress.com/2013/06/22/high-performance-content-defined-chunking/

A file archive stores a full directory tree. Content is stored using
the :ref:`pxar-format`, split into variable-sized chunks. The format
is optimized to achieve good deduplication rates.


Image Archives: ``<name>.img``
^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^

This is used for virtual machine images and other large binary
data. Content is split into fixed-sized chunks.


Binary Data (BLOBs)
^^^^^^^^^^^^^^^^^^^

This type is used to store smaller (< 16MB) binary data such as
configuration files. Larger files should be stored as image archive.

.. caution:: Please do not store all files as BLOBs. Instead, use the
   file archive to store whole directory trees.


Catalog File: ``catalog.pcat1``
^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^

The catalog file is an index for file archives. It contains
the list of files and is used to speed up search operations.


The Manifest: ``index.json``
^^^^^^^^^^^^^^^^^^^^^^^^^^^^

The manifest contains the list of all backup files, their
sizes and checksums. It is used to verify the consistency of a
backup.


Backup Type
~~~~~~~~~~~

The backup server groups backups by *type*, where *type* is one of:

``vm``
    This type is used for :term:`virtual machine`\ s. Typically
    consists of the virtual machine's configuration file and an image archive
    for each disk.

``ct``
    This type is used for :term:`container`\ s. Consists of the container's
    configuration and a single file archive for the filesystem content.

``host``
    This type is used for backups created from within the backed up machine.
    Typically this would be a physical host but could also be a virtual machine
    or container. Such backups may contain file and image archives, there are no restrictions in this regard.


Backup ID
~~~~~~~~~

A unique ID. Usually the virtual machine or container ID. ``host``
type backups normally use the hostname.


Backup Time
~~~~~~~~~~~

The time when the backup was made.


Backup Group
~~~~~~~~~~~~

The tuple ``<type>/<ID>`` is called a backup group. Such a group
may contain one or more backup snapshots.


Backup Snapshot
~~~~~~~~~~~~~~~

The triplet ``<type>/<ID>/<time>`` is called a backup snapshot. It
uniquely identifies a specific backup within a datastore.

.. code-block:: console
   :caption: Backup Snapshot Examples

    vm/104/2019-10-09T08:01:06Z
    host/elsa/2019-11-08T09:48:14Z

As you can see, the time format is RFC3399_ with Coordinated
Universal Time (UTC_, identified by the trailing *Z*).

Backup Server Management
------------------------

The command line tool to configure and manage the backup server is called
:command:`proxmox-backup-manager`.



:term:`DataStore`
~~~~~~~~~~~~~~~~~

A datastore is a place where backups are stored. The current implementation
uses a directory inside a standard unix file system (``ext4``, ``xfs``
or ``zfs``) to store the backup data.

Datastores are identified by a simple *ID*. You can configure it
when setting up the backup server.

.. note:: The `File Layout`_ requires the file system to support at least *65538*
   subdirectories per directory. That number comes from the 2\ :sup:`16`
   pre-created chunk namespace directories, and the ``.`` and ``..`` default
   directory entries. This requirement excludes certain filesystems and
   filesystem configuration from being supported for a datastore. For example,
   ``ext3`` as a whole or ``ext4`` with the ``dir_nlink`` feature manually disabled.


Datastore Configuration
~~~~~~~~~~~~~~~~~~~~~~~

You can configure multiple datastores. Minimum one datastore needs to be
configured. The datastore is identified by a simple `name` and points to a
directory on the filesystem. Each datastore also has associated retention
settings of how many backup snapshots for each interval of ``hourly``,
``daily``, ``weekly``, ``monthly``, ``yearly`` as well as a time-independent
number of backups to keep in that store. :ref:`Pruning <pruning>` and
:ref:`garbage collection <garbage-collection>` can also be configured to run
periodically based on a configured :term:`schedule` per datastore.

The following command creates a new datastore called ``store1`` on :file:`/backup/disk1/store1`

.. code-block:: console

  # proxmox-backup-manager datastore create store1 /backup/disk1/store1

To list existing datastores run:

.. code-block:: console

  # proxmox-backup-manager datastore list
  ┌────────┬──────────────────────┬─────────────────────────────┐
  │ name   │ path                 │ comment                     │
  ╞════════╪══════════════════════╪═════════════════════════════╡
  │ store1 │ /backup/disk1/store1 │ This is my default storage. │
  └────────┴──────────────────────┴─────────────────────────────┘

You can change settings of a datastore, for example to set a prune and garbage
collection schedule or retention settings using ``update`` subcommand and view
a datastore with the ``show`` subcommand:

.. code-block:: console

  # proxmox-backup-manager datastore update store1 --keep-last 7 --prune-schedule daily --gc-schedule 'Tue 04:27'
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

The `.chunks` directory contains folders, starting from `0000` and taking hexadecimal values until `ffff`. These
directories will store the chunked data after a backup operation has been executed.

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



User Management
~~~~~~~~~~~~~~~

Proxmox Backup Server supports several authentication realms, and you need to
choose the realm when you add a new user. Possible realms are:

:pam: Linux PAM standard authentication. Use this if you want to
      authenticate as Linux system user (Users need to exist on the
      system).

:pbs: Proxmox Backup Server realm. This type stores hashed passwords in
      ``/etc/proxmox-backup/shadow.json``.

After installation, there is a single user ``root@pam``, which
corresponds to the Unix superuser. You can use the
``proxmox-backup-manager`` command line tool to list or manipulate
users:

.. code-block:: console

  # proxmox-backup-manager user list
  ┌─────────────┬────────┬────────┬───────────┬──────────┬────────────────┬────────────────────┐
  │ userid      │ enable │ expire │ firstname │ lastname │ email          │ comment            │
  ╞═════════════╪════════╪════════╪═══════════╪══════════╪════════════════╪════════════════════╡
  │ root@pam    │      1 │        │           │          │                │ Superuser          │
  └─────────────┴────────┴────────┴───────────┴──────────┴────────────────┴────────────────────┘

The superuser has full administration rights on everything, so you
normally want to add other users with less privileges:

.. code-block:: console

  # proxmox-backup-manager user create john@pbs --email john@example.com

The create command lets you specify many options like ``--email`` or
``--password``. You can update or change any of them using the
update command later:

.. code-block:: console

  # proxmox-backup-manager user update john@pbs --firstname John --lastname Smith
  # proxmox-backup-manager user update john@pbs --comment "An example user."

.. todo:: Mention how to set password without passing plaintext password as cli argument.


The resulting user list looks like this:

.. code-block:: console

  # proxmox-backup-manager user list
  ┌──────────┬────────┬────────┬───────────┬──────────┬──────────────────┬──────────────────┐
  │ userid   │ enable │ expire │ firstname │ lastname │ email            │ comment          │
  ╞══════════╪════════╪════════╪═══════════╪══════════╪══════════════════╪══════════════════╡
  │ john@pbs │      1 │        │ John      │ Smith    │ john@example.com │ An example user. │
  ├──────────┼────────┼────────┼───────────┼──────────┼──────────────────┼──────────────────┤
  │ root@pam │      1 │        │           │          │                  │ Superuser        │
  └──────────┴────────┴────────┴───────────┴──────────┴──────────────────┴──────────────────┘

Newly created users do not have any permissions. Please read the next
section to learn how to set access permissions.

If you want to disable a user account, you can do that by setting ``--enable`` to ``0``

.. code-block:: console

  # proxmox-backup-manager user update john@pbs --enable 0

Or completely remove the user with:

.. code-block:: console

  # proxmox-backup-manager user remove john@pbs


Access Control
~~~~~~~~~~~~~~

By default new users do not have any permission. Instead you need to
specify what is allowed and what is not. You can do this by assigning
roles to users on specific objects like datastores or remotes. The
following roles exist:

**NoAccess**
  Disable Access - nothing is allowed.

**Admin**
  The Administrator can do anything.

**Audit**
  An Auditor can view things, but is not allowed to change settings.

**DatastoreAdmin**
  Can do anything on datastores.

**DatastoreAudit**
  Can view datastore settings and list content. But
  is not allowed to read the actual data.

**DataStoreReader**
  Can Inspect datastore content and can do restores.

**DataStoreBackup**
  Can backup and restore owned backups.

**DatastorePowerUser**
  Can backup, restore, and prune owned backups.

**RemoteAdmin**
  Can do anything on remotes.

**RemoteAudit**
  Can view remote settings.

**RemoteSyncOperator**
  Is allowed to read data from a remote.


:term:`Remote`
~~~~~~~~~~~~~~

A remote refers to a separate Proxmox Backup Server installation and a user on that
installation, from which you can `sync` datastores to a local datastore with a
`Sync Job`.

To add a remote, you need its hostname or ip, a userid and password on the
remote, and its certificate fingerprint. To get the fingerprint, use the
``proxmox-backup-manager cert info`` command on the remote.

.. code-block:: console

  # proxmox-backup-manager cert info |grep Fingerprint
  Fingerprint (sha256): 64:d3:ff:3a:50:38:53:5a:9b:f7:50:...:ab:fe

Using the information specified above, add the remote with:

.. code-block:: console

  # proxmox-backup-manager remote create pbs2 --host pbs2.mydomain.example --userid sync@pam --password 'SECRET' --fingerprint 64:d3:ff:3a:50:38:53:5a:9b:f7:50:...:ab:fe

Use the ``list``, ``show``, ``update``, ``remove`` subcommands of
``proxmox-backup-manager remote`` to manage your remotes:

.. code-block:: console

  # proxmox-backup-manager remote update pbs2 --host pbs2.example
  # proxmox-backup-manager remote list
  ┌──────┬──────────────┬──────────┬───────────────────────────────────────────┬─────────┐
  │ name │ host         │ userid   │ fingerprint                               │ comment │
  ╞══════╪══════════════╪══════════╪═══════════════════════════════════════════╪═════════╡
  │ pbs2 │ pbs2.example │ sync@pam │64:d3:ff:3a:50:38:53:5a:9b:f7:50:...:ab:fe │         │
  └──────┴──────────────┴──────────┴───────────────────────────────────────────┴─────────┘
  # proxmox-backup-manager remote remove pbs2


Sync Jobs
~~~~~~~~~

Sync jobs are configured to pull the contents of a datastore on a `Remote` to a
local datastore. You can either start the sync job manually on the GUI or
provide it with a :term:`schedule` to run regularly. The
``proxmox-backup-manager sync-job`` command is used to manage sync jobs:

.. code-block:: console

  # proxmox-backup-manager sync-job create pbs2-local --remote pbs2 --remote-store local --store local --schedule 'Wed 02:30'
  # proxmox-backup-manager sync-job update pbs2-local --comment 'offsite'
  # proxmox-backup-manager sync-job list
  ┌────────────┬───────┬────────┬──────────────┬───────────┬─────────┐
  │ id         │ store │ remote │ remote-store │ schedule  │ comment │
  ╞════════════╪═══════╪════════╪══════════════╪═══════════╪═════════╡
  │ pbs2-local │ local │ pbs2   │ local        │ Wed 02:30 │ offsite │
  └────────────┴───────┴────────┴──────────────┴───────────┴─────────┘
  # proxmox-backup-manager sync-job remove pbs2-local


Backup Client usage
-------------------

The command line client is called :command:`proxmox-backup-client`.


Repository Locations
~~~~~~~~~~~~~~~~~~~~

The client uses the following notation to specify a datastore repository
on the backup server.

  [[username@]server:]datastore

The default value for ``username`` ist ``root``.  If no server is specified,
the default is the local host (``localhost``).

You can pass the repository with the ``--repository`` command
line option, or by setting the ``PBS_REPOSITORY`` environment
variable.


Environment Variables
~~~~~~~~~~~~~~~~~~~~~

``PBS_REPOSITORY``
  The default backup repository.

``PBS_PASSWORD``
  When set, this value is used for the password required for the
  backup server.

``PBS_ENCRYPTION_PASSWORD``
  When set, this value is used to access the secret encryption key (if
  protected by password).

``PBS_FINGERPRINT`` When set, this value is used to verify the server
  certificate (only used if the system CA certificates cannot
  validate the certificate).


Output Format
~~~~~~~~~~~~~

Most commands support the ``--output-format`` parameter. It accepts
the following values:

:``text``: Text format (default). Structured data is rendered as a table.

:``json``: JSON (single line).

:``json-pretty``: JSON (multiple lines, nicely formatted).


Please use the following environment variables to modify output behavior:

``PROXMOX_OUTPUT_FORMAT``
  Defines the default output format.

``PROXMOX_OUTPUT_NO_BORDER``
  If set (to any value), do not render table borders.

``PROXMOX_OUTPUT_NO_HEADER``
  If set (to any value), do not render table headers.

.. note:: The ``text`` format is designed to be human readable, and
   not meant to be parsed by automation tools. Please use the ``json``
   format if you need to process the output.


.. _creating-backups:

Creating Backups
~~~~~~~~~~~~~~~~

This section explains how to create a backup from within the machine. This can
be a physical host, a virtual machine, or a container. Such backups may contain file
and image archives. There are no restrictions in this case.

.. note:: If you want to backup virtual machines or containers on Proxmox VE, see :ref:`pve-integration`.

For the following example you need to have a backup server set up, working
credentials and need to know the repository name.
In the following examples we use ``backup-server:store1``.

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

.. Caution:: Please note that the proxmox-backup-client does not
   automatically include mount points. Instead, you will see a short
   ``skip mount point`` notice for each of them. The idea is to
   create a separate file archive for each mounted disk. You can
   explicitly include them using the ``--include-dev`` option
   (i.e. ``--include-dev /boot/efi``). You can use this option
   multiple times for each mount point that should be included.

The ``--repository`` option can get quite long and is used by all
commands. You can avoid having to enter this value by setting the
environment variable ``PBS_REPOSITORY``.

.. code-block:: console

  # export PBS_REPOSITORY=backup-server:store1

After this you can execute all commands without specifying the ``--repository``
option.

One single backup is allowed to contain more than one archive. For example, if
you want to backup two disks mounted at ``/mmt/disk1`` and ``/mnt/disk2``:

.. code-block:: console

  # proxmox-backup-client backup disk1.pxar:/mnt/disk1 disk2.pxar:/mnt/disk2

This creates a backup of both disks.

The backup command takes a list of backup specifications, which
include the archive name on the server, the type of the archive, and the
archive source at the client. The format is:

    <archive-name>.<type>:<source-path>

Common types are ``.pxar`` for file archives, and ``.img`` for block
device images. To create a backup of a block device run the following command:

.. code-block:: console

  # proxmox-backup-client backup mydata.img:/dev/mylvm/mydata

Excluding files/folders from a backup
^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^

Sometimes it is desired to exclude certain files or folders from a backup archive.
To tell the Proxmox backup client when and how to ignore files and directories,
place a text file called ``.pxarexclude`` in the filesystem hierarchy.
Whenever the backup client encounters such a file in a directory, it interprets
each line as glob match patterns for files and directories that are to be excluded
from the backup.

The file must contain a single glob pattern per line. Empty lines are ignored.
The same is true for lines starting with ``#``, which indicates a comment.
A ``!`` at the beginning of a line reverses the glob match pattern from an exclusion
to an explicit inclusion. This makes it possible to exclude all entries in a
directory except for a few single files/subdirectories.
Lines ending in ``/`` match only on directories.
The directory containing the ``.pxarexclude`` file is considered to be the root of
the given patterns. It is only possible to match files in this directory and its subdirectories.

``\`` is used to escape special glob characters.
``?`` matches any single character.
``*`` matches any character, including an empty string.
``**`` is used to match subdirectories. It can be used to, for example, exclude
all files ending in ``.tmp`` within the directory or subdirectories with the
following pattern ``**/*.tmp``.
``[...]`` matches a single character from any of the provided characters within
the brackets. ``[!...]`` does the complementary and matches any single character
not contained within the brackets. It is also possible to specify ranges with two
characters separated by ``-``. For example, ``[a-z]`` matches any lowercase
alphabetic character and ``[0-9]`` matches any one single digit.

The order of the glob match patterns defines whether a file is included or
excluded, that is to say later entries override previous ones.
This is also true for match patterns encountered deeper down the directory tree,
which can override a previous exclusion.
Be aware that excluded directories will **not** be read by the backup client.
Thus, a ``.pxarexclude`` file in an excluded subdirectory will have no effect.
``.pxarexclude`` files are treated as regular files and will be included in the
backup archive.

For example, consider the following directory structure:

.. code-block:: console

    # ls -aR folder
    folder/:
    .  ..  .pxarexclude  subfolder0  subfolder1

    folder/subfolder0:
    .  ..  file0  file1  file2  file3  .pxarexclude

    folder/subfolder1:
    .  ..  file0  file1  file2  file3

The different ``.pxarexclude`` files contain the following:

.. code-block:: console

    # cat folder/.pxarexclude
    /subfolder0/file1
    /subfolder1/*
    !/subfolder1/file2

.. code-block:: console

    # cat folder/subfolder0/.pxarexclude
    file3

This would exclude ``file1`` and ``file3`` in ``subfolder0`` and all of
``subfolder1`` except ``file2``.

Restoring this backup will result in:

.. code-block:: console

    ls -aR restored
    restored/:
    .  ..  .pxarexclude  subfolder0  subfolder1

    restored/subfolder0:
    .  ..  file0  file2  .pxarexclude

    restored/subfolder1:
    .  ..  file2

Encryption
^^^^^^^^^^

Proxmox Backup supports client-side encryption with AES-256 in GCM_
mode. To set this up, you first need to create an encryption key:

.. code-block:: console

  # proxmox-backup-client key create my-backup.key
  Encryption Key Password: **************

The key is password protected by default. If you do not need this
extra protection, you can also create it without a password:

.. code-block:: console

  # proxmox-backup-client key create /path/to/my-backup.key --kdf none


.. code-block:: console

  # proxmox-backup-client backup etc.pxar:/etc --keyfile /path/to/my-backup.key
  Password: *********
  Encryption Key Password: **************
  ...


You can avoid entering the passwords by setting the environment
variables ``PBS_PASSWORD`` and ``PBS_ENCRYPTION_PASSWORD``.

.. todo:: Explain master-key


Restoring Data
~~~~~~~~~~~~~~

The regular creation of backups is a necessary step to avoiding data
loss. More importantly, however, is the restoration. It is good practice to perform
periodic recovery tests to ensure that you can access the data in
case of problems.

First, you need to find the snapshot which you want to restore. The snapshot
command provides a list of all the snapshots on the server:

.. code-block:: console

  # proxmox-backup-client snapshots
  ┌────────────────────────────────┬─────────────┬────────────────────────────────────┐
  │ snapshot                       │        size │ files                              │
  ╞════════════════════════════════╪═════════════╪════════════════════════════════════╡
  │ host/elsa/2019-12-03T09:30:15Z │ 51788646825 │ root.pxar catalog.pcat1 index.json │
  ├────────────────────────────────┼─────────────┼────────────────────────────────────┤
  │ host/elsa/2019-12-03T09:35:01Z │ 51790622048 │ root.pxar catalog.pcat1 index.json │
  ├────────────────────────────────┼─────────────┼────────────────────────────────────┤
  ...

You can inspect the catalog to find specific files.

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

To get the contents of any archive, you can restore the ``ìndex.json`` file in the
repository to the target path '-'. This will dump the contents to the standard output.

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
utilizes the metadata stored in the catalog to quickly list, navigate and
search files in a file archive.
To restore files, you can select them individually or match them with a glob
pattern.

Using the catalog for navigation reduces the overhead considerably because only
the catalog needs to be downloaded and, optionally, decrypted.
The actual chunks are only accessed if the metadata in the catalog is not enough
or for the actual restore.

Similar to common UNIX shells ``cd`` and ``ls`` are the commands used to change
working directory and list directory contents in the archive.
``pwd`` shows the full path of the current working directory with respect to the
archive root.

Being able to quickly search the contents of the archive is a commmonly needed feature.
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

This allows you to access the full contents of the archive in a seamless manner.

.. note:: As the FUSE connection needs to fetch and decrypt chunks from the
    backup server's datastore, this can cause some additional network and CPU
    load on your host, depending on the operations you perform on the mounted
    filesystem.

To unmount the filesystem use the ``umount`` command on the mountpoint:

.. code-block:: console

  # umount /mnt

Login and Logout
~~~~~~~~~~~~~~~~

The client tool prompts you to enter the logon password as soon as you
want to access the backup server. The server checks your credentials
and responds with a ticket that is valid for two hours. The client
tool automatically stores that ticket and uses it for further requests
to this server.

You can also manually trigger this login/logout using the login and
logout commands:

.. code-block:: console

  # proxmox-backup-client login
  Password: **********

To remove the ticket, issue a logout:

.. code-block:: console

  # proxmox-backup-client logout


.. _pruning:

Pruning and Removing Backups
~~~~~~~~~~~~~~~~~~~~~~~~~~~~

You can manually delete a backup snapshot using the ``forget``
command:

.. code-block:: console

  # proxmox-backup-client forget <snapshot>


.. caution:: This command removes all archives in this backup
   snapshot. They will be inaccessible and unrecoverable.


The manual removal is sometimes required, but normally the prune
command is used to systematically delete older backups. Prune lets
you specify which backup snapshots you want to keep. The
following retention options are available:

``--keep-last <N>``
  Keep the last ``<N>`` backup snapshots.

``--keep-hourly <N>``
  Keep backups for the last ``<N>`` hours. If there is more than one
  backup for a single hour, only the latest is kept.

``--keep-daily <N>``
  Keep backups for the last ``<N>`` days. If there is more than one
  backup for a single day, only the latest is kept.

``--keep-weekly <N>``
  Keep backups for the last ``<N>`` weeks. If there is more than one
  backup for a single week, only the latest is kept.

  .. note:: Weeks start on Monday and end on Sunday. The software
     uses the `ISO week date`_ system and handles weeks at
     the end of the year correctly.

``--keep-monthly <N>``
  Keep backups for the last ``<N>`` months. If there is more than one
  backup for a single month, only the latest is kept.

``--keep-yearly <N>``
  Keep backups for the last ``<N>`` years. If there is more than one
  backup for a single year, only the latest is kept.

The retention options are processed in the order given above. Each option
only covers backups within its time period. The next option does not take care
of already covered backups. It will only consider older backups.

Unfinished and incomplete backups will be removed by the prune command unless
they are newer than the last successful backup. In this case, the last failed
backup is retained.

.. code-block:: console

  # proxmox-backup-client prune <group> --keep-daily 7 --keep-weekly 4 --keep-monthly 3


You can use the ``--dry-run`` option to test your settings. This only
shows the list of existing snapshots and what actions prune would take.

.. code-block:: console

  # proxmox-backup-client prune host/elsa --dry-run --keep-daily 1 --keep-weekly 3
  ┌────────────────────────────────┬──────┐
  │ snapshot                       │ keep │
  ╞════════════════════════════════╪══════╡
  │ host/elsa/2019-12-04T13:20:37Z │    1 │
  ├────────────────────────────────┼──────┤
  │ host/elsa/2019-12-03T09:35:01Z │    0 │
  ├────────────────────────────────┼──────┤
  │ host/elsa/2019-11-22T11:54:47Z │    1 │
  ├────────────────────────────────┼──────┤
  │ host/elsa/2019-11-21T12:36:25Z │    0 │
  ├────────────────────────────────┼──────┤
  │ host/elsa/2019-11-10T10:42:20Z │    1 │
  └────────────────────────────────┴──────┘

.. note:: Neither the ``prune`` command nor the ``forget`` command free space
   in the chunk-store. The chunk-store still contains the data blocks. To free
   space you need to perform :ref:`garbage-collection`.


.. _garbage-collection:

Garbage Collection
~~~~~~~~~~~~~~~~~~

The ``prune`` command removes only the backup index files, not the data
from the data store. This task is left to the garbage collection
command. It is recommended to carry out garbage collection on a regular basis.

The garbage collection works in two phases. In the first phase, all
data blocks that are still in use are marked. In the second phase,
unused data blocks are removed.

.. note:: This command needs to read all existing backup index files
  and touches the complete chunk-store. This can take a long time
  depending on the number of chunks and the speed of the underlying
  disks.

.. note:: The garbage collection will only remove chunks that haven't been used
   for at least one day (exactly 24h 5m). This grace period is necessary because
   chunks in use are marked by touching the chunk which updates the ``atime``
   (access time) property. Filesystems are mounted with the ``relatime`` option
   by default. This results in a better performance by only updating the
   ``atime`` property if the last access has been at least 24 hours ago. The
   downside is, that touching a chunk within these 24 hours will not always
   update its ``atime`` property.

   Chunks in the grace period will be logged at the end of the garbage
   collection task as *Pending removals*.

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

You need to define a new storage with type 'pbs' on your `Proxmox VE`_
node. The following example uses ``store2`` as storage name, and
assumes the server address is ``localhost``, and you want to connect
as ``user1@pbs``.

.. code-block:: console

  # pvesm add pbs store2 --server localhost --datastore store2
  # pvesm set store2 --username user1@pbs --password <secret>

If your backup server uses a self signed certificate, you need to add
the certificate fingerprint to the configuration. You can get the
fingerprint by running the following command on the backup server:

.. code-block:: console

  # proxmox-backup-manager cert  info |grep Fingerprint
  Fingerprint (sha256): 64:d3:ff:3a:50:38:53:5a:9b:f7:50:...:ab:fe

Please add that fingerprint to your configuration to establish a trust
relationship:

.. code-block:: console

  # pvesm set store2 --fingerprint  64:d3:ff:3a:50:38:53:5a:9b:f7:50:...:ab:fe

After that you should be able to see storage status with:

.. code-block:: console

  # pvesm status --storage store2
  Name             Type     Status           Total            Used       Available        %
  store2            pbs     active      3905109820      1336687816      2568422004   34.23%



.. include:: command-line-tools.rst

.. include:: services.rst
