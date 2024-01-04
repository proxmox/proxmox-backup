Managing Remotes & Sync
=======================

.. _backup_remote:

:term:`Remote`
--------------

A remote refers to a separate `Proxmox Backup`_ Server installation and a user
on that installation, from which you can `sync` datastores to a local datastore
with a `Sync Job`. You can configure remotes in the web interface, under
**Configuration -> Remotes**. Alternatively, you can use the ``remote``
subcommand. The configuration information for remotes is stored in the file
``/etc/proxmox-backup/remote.cfg``.

.. image:: images/screenshots/pbs-gui-remote-add.png
  :target: _images/pbs-gui-remote-add.png
  :align: right
  :alt: Add a remote

To add a remote, you need its hostname or IP address, a userid and password on
the remote, and its certificate fingerprint. To get the fingerprint, use the
``proxmox-backup-manager cert info`` command on the remote, or navigate to
**Dashboard** in the remote's web interface and select **Show Fingerprint**.

.. code-block:: console

  # proxmox-backup-manager cert info |grep Fingerprint
  Fingerprint (sha256): 64:d3:ff:3a:50:38:53:5a:9b:f7:50:...:ab:fe

Using the information specified above, you can add a remote from the **Remotes**
configuration panel, or by using the command:

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


.. _syncjobs:

Sync Jobs
---------

.. image:: images/screenshots/pbs-gui-syncjob-add.png
  :target: _images/pbs-gui-syncjob-add.png
  :align: right
  :alt: Add a Sync Job

Sync jobs are configured to pull the contents of a datastore on a **Remote** to
a local datastore. You can manage sync jobs in the web interface, from the
**Sync Jobs** tab of the **Datastore** panel or from that of the Datastore
itself. Alternatively, you can manage them with the ``proxmox-backup-manager
sync-job`` command. The configuration information for sync jobs is stored at
``/etc/proxmox-backup/sync.cfg``. To create a new sync job, click the add button
in the GUI, or use the ``create`` subcommand. After creating a sync job, you can
either start it manually from the GUI or provide it with a schedule (see
:ref:`calendar-event-scheduling`) to run regularly.

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

To set up sync jobs, the configuring user needs the following permissions:

#. ``Remote.Read`` on the ``/remote/{remote}/{remote-store}`` path
#. At least ``Datastore.Backup`` on the local target datastore (``/datastore/{store}``)

.. note:: A sync job can only sync backup groups that the configured remote's
  user/API token can read. If a remote is configured with a user/API token that
  only has ``Datastore.Backup`` privileges, only the limited set of accessible
  snapshots owned by that user/API token can be synced.

If the ``remove-vanished`` option is set, ``Datastore.Prune`` is required on
the local datastore as well. If the ``owner`` option is not set (defaulting to
``root@pam``) or is set to something other than the configuring user,
``Datastore.Modify`` is required as well.

If the ``group-filter`` option is set, only backup groups matching at least one
of the specified criteria are synced. The available criteria are:

* Backup type, for example, to only sync groups of the `ct` (Container) type:
    .. code-block:: console

     # proxmox-backup-manager sync-job update ID --group-filter type:ct
* Full group identifier, to sync a specific backup group:
    .. code-block:: console

     # proxmox-backup-manager sync-job update ID --group-filter group:vm/100
* Regular expression, matched against the full group identifier
    .. code-block:: console

     # proxmox-backup-manager sync-job update ID --group-filter regex:'^vm/1\d{2,3}$'

The same filter is applied to local groups, for handling of the
``remove-vanished`` option.

A ``group-filter`` can be inverted by prepending ``exclude:`` to it.

* Regular expression example, excluding the match:
    .. code-block:: console

     # proxmox-backup-manager sync-job update ID --group-filter exclude:regex:'^vm/1\d{2,3}$'

For mixing include and exclude filter, following rules apply:

 - no filters: all backup groups
 - include: only those matching the include filters
 - exclude: all but those matching the exclude filters
 - both: those matching the include filters, but without those matching the exclude filters
.. note:: The ``protected`` flag of remote backup snapshots will not be synced.

Namespace Support
^^^^^^^^^^^^^^^^^

Sync jobs can be configured to not only sync datastores, but also subsets of
datastores in the form of namespaces or namespace sub-trees. The following
parameters influence how namespaces are treated as part of a sync job's
execution:

- ``remote-ns``: the remote namespace anchor (default: the root namespace)

- ``ns``: the local namespace anchor (default: the root namespace)

- ``max-depth``: whether to recursively iterate over sub-namespaces of the remote
  namespace anchor (default: `None`)

If ``max-depth`` is set to `0`, groups are synced from ``remote-ns`` into
``ns``, without any recursion. If it is set to `None` (left empty), recursion
depth will depend on the value of ``remote-ns`` and the remote side's
availability of namespace support:

- ``remote-ns`` set to something other than the root namespace: remote *must*
  support namespaces, full recursion starting at ``remote-ns``.

- ``remote-ns`` set to root namespace and remote *supports* namespaces: full
  recursion starting at root namespace.

- ``remote-ns`` set to root namespace and remote *does not support* namespaces:
  backwards-compat mode, only root namespace will be synced into ``ns``, no
  recursion.

Any other value of ``max-depth`` will limit recursion to at most ``max-depth``
levels, for example: ``remote-ns`` set to `location_a/department_b` and
``max-depth`` set to `1` will result in `location_a/department_b` and at most
one more level of sub-namespaces being synced.

The namespace tree starting at ``remote-ns`` will be mapped into ``ns`` up to a
depth of ``max-depth``.

For example, with the following namespaces at the remote side:

- `location_a`

  - `location_a/department_x`

    - `location_a/department_x/team_one`

    - `location_a/department_x/team_two`

  - `location_a/department_y`

    - `location_a/department_y/team_one`

    - `location_a/department_y/team_two`

- `location_b`

and ``remote-ns`` being set to `location_a/department_x` and ``ns`` set to
`location_a_dep_x` resulting in the following namespace tree on the sync
target:

- `location_a_dep_x` (containing the remote's `location_a/department_x`)

  - `location_a_dep_x/team_one` (containing the remote's `location_a/department_x/team_one`)

  - `location_a_dep_x/team_two` (containing the remote's `location_a/department_x/team_two`)

with the rest of the remote namespaces and groups not being synced (by this
sync job).

If a remote namespace is included in the sync job scope, but does not exist
locally, it will be created (provided the sync job owner has sufficient
privileges).

If the ``remove-vanished`` option is set, namespaces that are included in the
sync job scope but only exist locally are treated as vanished and removed
(provided the sync job owner has sufficient privileges).

.. note:: All other limitations on sync scope (such as remote user/API token
   privileges, group filters) also apply for sync jobs involving one or
   multiple namespaces.

Bandwidth Limit
^^^^^^^^^^^^^^^

Syncing a datastore to an archive can produce a lot of traffic and impact other
users of the network. In order to avoid network or storage congestion, you can
limit the bandwidth of the sync job by setting the ``rate-in`` option either in
the web interface or using the ``proxmox-backup-manager`` command-line tool:

.. code-block:: console

    # proxmox-backup-manager sync-job update ID --rate-in 20MiB
