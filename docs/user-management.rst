.. _user_mgmt:

User Management
===============


User Configuration
------------------

.. image:: images/screenshots/pbs-gui-user-management.png
  :align: right
  :alt: User management

Proxmox Backup Server supports several authentication realms, and you need to
choose the realm when you add a new user. Possible realms are:

:pam: Linux PAM standard authentication. Use this if you want to
      authenticate as Linux system user (Users need to exist on the
      system).

:pbs: Proxmox Backup Server realm. This type stores hashed passwords in
      ``/etc/proxmox-backup/shadow.json``.

After installation, there is a single user ``root@pam``, which
corresponds to the Unix superuser. User configuration information is stored in the file
``/etc/proxmox-backup/user.cfg``. You can use the
``proxmox-backup-manager`` command line tool to list or manipulate
users:

.. code-block:: console

  # proxmox-backup-manager user list
  ┌─────────────┬────────┬────────┬───────────┬──────────┬────────────────┬────────────────────┐
  │ userid      │ enable │ expire │ firstname │ lastname │ email          │ comment            │
  ╞═════════════╪════════╪════════╪═══════════╪══════════╪════════════════╪════════════════════╡
  │ root@pam    │      1 │        │           │          │                │ Superuser          │
  └─────────────┴────────┴────────┴───────────┴──────────┴────────────────┴────────────────────┘

.. image:: images/screenshots/pbs-gui-user-management-add-user.png
  :align: right
  :alt: Add a new user

The superuser has full administration rights on everything, so you
normally want to add other users with less privileges. You can create a new
user with the ``user create`` subcommand or through the web interface, under
**Configuration -> User Management**. The ``create`` subcommand lets you specify
many options like ``--email`` or ``--password``. You can update or change any
user properties using the ``update`` subcommand later (**Edit** in the GUI):


.. code-block:: console

  # proxmox-backup-manager user create john@pbs --email john@example.com
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


.. _user_acl:

Access Control
--------------

By default new users do not have any permission. Instead you need to
specify what is allowed and what is not. You can do this by assigning
roles to users on specific objects like datastores or remotes. The
following roles exist:

**NoAccess**
  Disable Access - nothing is allowed.

**Admin**
  Can do anything.

**Audit**
  Can view things, but is not allowed to change settings.

**DatastoreAdmin**
  Can do anything on datastores.

**DatastoreAudit**
  Can view datastore settings and list content. But
  is not allowed to read the actual data.

**DatastoreReader**
  Can Inspect datastore content and can do restores.

**DatastoreBackup**
  Can backup and restore owned backups.

**DatastorePowerUser**
  Can backup, restore, and prune owned backups.

**RemoteAdmin**
  Can do anything on remotes.

**RemoteAudit**
  Can view remote settings.

**RemoteSyncOperator**
  Is allowed to read data from a remote.

.. image:: images/screenshots/pbs-gui-permissions-add.png
  :align: right
  :alt: Add permissions for user

Access permission information is stored in ``/etc/proxmox-backup/acl.cfg``. The
file contains 5 fields, separated using a colon (':') as a delimiter. A typical
entry takes the form:

``acl:1:/datastore:john@pbs:DatastoreBackup``

The data represented in each field is as follows:

#. ``acl`` identifier
#. A ``1`` or ``0``, representing whether propagation is enabled or disabled,
   respectively
#. The object on which the permission is set. This can be a specific object
   (single datastore, remote, etc.) or a top level object, which with
   propagation enabled, represents all children of the object also.
#. The user for which the permission is set
#. The role being set

You can manage datastore permissions from **Configuration -> Permissions** in the
web interface. Likewise, you can use the ``acl`` subcommand to manage and
monitor user permissions from the command line. For example, the command below
will add the user ``john@pbs`` as a **DatastoreAdmin** for the datastore
``store1``, located at ``/backup/disk1/store1``:

.. code-block:: console

  # proxmox-backup-manager acl update /datastore/store1 DatastoreAdmin --userid john@pbs

You can monitor the roles of each user using the following command:

.. code-block:: console

   # proxmox-backup-manager acl list
   ┌──────────┬──────────────────┬───────────┬────────────────┐
   │ ugid     │ path             │ propagate │ roleid         │
   ╞══════════╪══════════════════╪═══════════╪════════════════╡
   │ john@pbs │ /datastore/disk1 │         1 │ DatastoreAdmin │
   └──────────┴──────────────────┴───────────┴────────────────┘

A single user can be assigned multiple permission sets for different datastores.

.. Note::
  Naming convention is important here. For datastores on the host,
  you must use the convention ``/datastore/{storename}``. For example, to set
  permissions for a datastore mounted at ``/mnt/backup/disk4/store2``, you would use
  ``/datastore/store2`` for the path. For remote stores, use the convention
  ``/remote/{remote}/{storename}``, where ``{remote}`` signifies the name of the
  remote (see `Remote` below) and ``{storename}`` is the name of the datastore on
  the remote.


