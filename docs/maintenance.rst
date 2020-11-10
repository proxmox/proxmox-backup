Maintenance Tasks
=================

.. _maintenance_pruning:

Pruning
-------

Prune lets you specify which backup snapshots you want to keep. The
following retention options are available:

``keep-last <N>``
  Keep the last ``<N>`` backup snapshots.

``keep-hourly <N>``
  Keep backups for the last ``<N>`` hours. If there is more than one
  backup for a single hour, only the latest is kept.

``keep-daily <N>``
  Keep backups for the last ``<N>`` days. If there is more than one
  backup for a single day, only the latest is kept.

``keep-weekly <N>``
  Keep backups for the last ``<N>`` weeks. If there is more than one
  backup for a single week, only the latest is kept.

  .. note:: Weeks start on Monday and end on Sunday. The software
     uses the `ISO week date`_ system and handles weeks at
     the end of the year correctly.

``keep-monthly <N>``
  Keep backups for the last ``<N>`` months. If there is more than one
  backup for a single month, only the latest is kept.

``keep-yearly <N>``
  Keep backups for the last ``<N>`` years. If there is more than one
  backup for a single year, only the latest is kept.

The retention options are processed in the order given above. Each option
only covers backups within its time period. The next option does not take care
of already covered backups. It will only consider older backups.

Unfinished and incomplete backups will be removed by the prune command unless
they are newer than the last successful backup. In this case, the last failed
backup is retained.

Prune Simulator
^^^^^^^^^^^^^^^

You can use the built-in `prune simulator <prune-simulator/index.html>`_
to explore the effect of different retetion options with various backup
schedules.

Manual Pruning
^^^^^^^^^^^^^^

.. image:: images/screenshots/pbs-gui-datastore-content-prune-group.png
  :target: _images/pbs-gui-datastore-content-prune-group.png
  :align: right
  :alt: Prune and garbage collection options

To access pruning functionality for a specific backup group, you can use the
prune command line option discussed in :ref:`backup-pruning`, or navigate to
the **Content** tab of the datastore and click the scissors icon in the
**Actions** column of the relevant backup group.

Prune Schedules
^^^^^^^^^^^^^^^

To prune on a datastore level, scheduling options can be found under the
**Prune & GC** tab of the datastore. Here you can set retention settings and
edit the interval at which pruning takes place.

.. image:: images/screenshots/pbs-gui-datastore-prunegc.png
  :target: _images/pbs-gui-datastore-prunegc.png
  :align: right
  :alt: Prune and garbage collection options


.. _maintenance_gc:

Garbage Collection
------------------

You can monitor and run :ref:`garbage collection <garbage-collection>` on the
Proxmox Backup Server using the ``garbage-collection`` subcommand of
``proxmox-backup-manager``. You can use the ``start`` subcommand to manually
start garbage collection on an entire datastore and the ``status`` subcommand to
see attributes relating to the :ref:`garbage collection <garbage-collection>`.

This functionality can also be accessed in the GUI, by navigating to **Prune &
GC** from the top panel. From here, you can edit the schedule at which garbage
collection runs and manually start the operation.


.. _maintenance_verification:

Verification
------------

.. image:: images/screenshots/pbs-gui-datastore-verifyjob-add.png
  :target: _images/pbs-gui-datastore-verifyjob-add.png
  :align: right
  :alt: Adding a verify job

Proxmox Backup offers various verification options to ensure that backup data is
intact.  Verification is generally carried out through the creation of verify
jobs. These are scheduled tasks that run verification at a given interval (see
:ref:`calendar-events`). With these, you can set whether already verified
snapshots are ignored, as well as set a time period, after which verified jobs
are checked again. The interface for creating verify jobs can be found under the
**Verify Jobs** tab of the datastore.

.. Note:: It is recommended that you reverify all backups at least monthly, even
  if a previous verification was successful. This is becuase physical drives
  are susceptible to damage over time, which can cause an old, working backup
  to become corrupted in a process known as `bit rot/data degradation
  <https://en.wikipedia.org/wiki/Data_degradation>`_. It is good practice to
  have a regularly recurring (hourly/daily) verification job, which checks new
  and expired backups, then another weekly/monthly job that will reverify
  everything. This way, there will be no surprises when it comes to restoring
  data.

Aside from using verify jobs, you can also run verification manually on entire
datastores, backup groups, or snapshots. To do this, navigate to the **Content**
tab of the datastore and either click *Verify All*, or select the *V.* icon from
the *Actions* column in the table.

.. _maintenance_notification:

Notifications
-------------

Proxmox Backup Server can send you notification emails about automatically
scheduled verification, garbage-collection and synchronization tasks results.

By default, notifications are send to the email address configured for the
`root@pam` user. You can set that user for each datastore.

You can also change the level of notification received per task type, the
following options are available:

* Always: send a notification for any scheduled task, independent of the
  outcome

* Errors: send a notification for any scheduled task resulting in an error

* Never: do not send any notification at all
