Administration Guide
====================

The administartion guide.


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
