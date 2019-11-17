Administration Guide
====================

The administartion guide.


Terminology
-----------

Backup Type
~~~~~~~~~~~

The backup server groups backups by *type*, where *type* is one of:

``vm``
    This type is use for :term:`virtual machine`\ s. Typically
    contains the virtual machine configuration and an image archive
    for each disk.

``ct``
    This type is use for :term:`container`\ s. Contains the container
    configuration and a single file archive for the container content.

``host``
    This type is used for physical host, or if you want to run backup
    manually from inside virtual machines or containers. Such backup
    may contains file and image archives (no restrictions here).


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


Encryption
^^^^^^^^^^


Restoring Data
~~~~~~~~~~~~~~


`Proxmox VE`_ integration
-------------------------


.. include:: command-line-tools.rst

.. include:: services.rst

.. include host system admin at the end

.. include:: sysadmin.rst
