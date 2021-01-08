Tape Backup
===========

Our tape backup solution provides a easy way to store datastore
contents to a tape. This increases data safety because you get:

- an additional copy of the data
- to a different media type (tape)
- to an additional location (you can move tape offsite)

Statistics show that 95% of all restore jobs restores the last
backup. Restore requests further declines the older the data gets.
Considering that, tape backup may also help to reduce disk usage,
because you can safely remove data from disk once archived on tape.
This is especially true if you need to keep data for several years.

Tape backups do not provide random access to the stored data. Instead,
you need to restore the data to disk before you can access it
again. Also, if you store your tapes offsite (using some kind of tape
vaulting service), you need to bring them onsite before you can do any
restore. So please consider that restores from tapes can take much
longer than restores from disk.


Tape Technology Primer
----------------------

.. _Linear Tape Open: https://en.wikipedia.org/wiki/Linear_Tape-Open

As of 2021, the only broadly available tape technology standard is
`Linear Tape Open`_, and different vendors offers LTO Ultrium tape
drives, autoloaders and LTO tape cartridges.

Of cause, there are a few vendor offering proprietary drives with
slight advantages in performance and capacity, but they have
significat disadvantages:

- proprietary (single vendor)
- a much higher purchase cost

So we currently do no test such drives.

In general, LTO tapes offer the following advantages:

- Durable (30 years)
- High Capacity (12 TB)
- Relatively low cost per TB
- Cold Media
- Movable (storable inside vault)
- Multiple vendors (for both media and drives)

Please note that `Proxmox Backup Server` already stores compressed
data, so we do not need/use the tape compression feature. Same applies
to encryption.


Supported Hardware
------------------

Proxmox Backup Server supports `Linear Tape Open`_ genertion 3
(LTO3) or later. In general, all SCSI2 tape drives supported by
the Linux kernel should work.

Tape changer support is done using the Linux 'mtx' command line
tool. So any changer devive supported by that tool work work.


Drive Performance
~~~~~~~~~~~~~~~~~

Current LTO8 tapes provide read/write speeds up to 360MB/s. Please
note that it still takes a minimum of 9 hours to completely write or
read a single tape (even at maximum speed).

The only way to speed up that data rate is to use more than one
drive. That way you can run several backup jobs in parallel, or run
restore jobs while the other dives are used for backups.

Also consider that you need to read data first from your datastore
(disk). But a single spinning disk is unable to deliver data at this
rate. We meassured a maximum rate about 100MB/s in practive, so it
takes 33 hours to read 12TB to fill up a LTO8 tape. So if you want to
run your tape at full speed, please make sure that the source
datastore is able to delive that performance (use SSDs).


Terminology
-----------

:Tape Labels: are used to uniquely indentify a tape. You normally use
   some sticky paper labels and apply them on the front of the
   cartridge. We additionally store the label text magnetically on the
   tape (first file on tape).

.. _Code 39: https://en.wikipedia.org/wiki/Code_39

.. _LTO Ultrium Cartridge Label Specification: https://www.ibm.com/support/pages/ibm-lto-ultrium-cartridge-label-specification

.. _LTO Barcode Generator: lto-barcode/index.html

:Barcodes: are a special form of tape labels, which are electronically
   readable. Most LTO tape robots use an 8 character string encoded as
   `Code 39`_, as definded in the `LTO Ultrium Cartridge Label
   Specification`_.

   You can either bye such barcode labels from your cartidge vendor,
   or print them yourself. You can use our `LTO Barcode Generator`_ App
   for that.

.. Note:: Physical labels and the associated adhesive shall have an
   environmental performance to match or exceed the environmental
   specifications of the cartridge to which it is applied.

:Media Pools: A media pool is a logical container for tapes. A backup
   job targets one media pool, so a job only uses tapes from that
   pool. The pool additionally defines how long a backup job can
   append data to tapes (allocation policy) and how long you want to
   keep the data (retention policy).

:Media Set: A group of continuously written tapes (all from the same
   media pool).

:Tape drive: The decive used to read and write data to the tape. There
   are standalone drives, but drives often ship within tape libraries.

:Tape changer: A device which can change the tapes inside a tape drive
   (tape robot). They are usually part of a tape library.

.. _Tape Library: https://en.wikipedia.org/wiki/Tape_library

:`Tape library`_: A storage device that contains one or more tape drives,
   a number of slots to hold tape cartridges, a barcode reader to
   identify tape cartridges and an automated method for loading tapes
   (a robot).

   People als call this 'autoloader', 'tape robot' or 'tape jukebox'.


Tape Quickstart
---------------

1. Configure your tape hardware (drives and changers)

2. Configure one or more media pools

3. Label your tape cartridges.

4. Start your first tape backup job ...


Configuration
-------------

Please note that you can configure anything using the graphical user
interface or the command line interface. Both methods results in the
same configuration.


Tape changers
~~~~~~~~~~~~~

Tape changers (robots) are part of a `Tape Library`_. You can skip
this step if you are using a standalone drive.

Linux is able to auto detect those devices, and you can get a list
of available devices using::

 # proxmox-tape changer scan
 ┌─────────────────────────────┬─────────┬──────────────┬────────┐
 │ path                        │ vendor  │ model        │ serial │
 ╞═════════════════════════════╪═════════╪══════════════╪════════╡
 │ /dev/tape/by-id/scsi-CC2C52 │ Quantum │ Superloader3 │ CC2C52 │
 └─────────────────────────────┴─────────┴──────────────┴────────┘

In order to use that device with Proxmox, you need to create a
configuration entry::

 # proxmox-tape changer create sl3 --path /dev/tape/by-id/scsi-CC2C52

Where ``sl3`` is an arbitrary name you can choose.

.. Note:: Please use stable device path names from inside
   ``/dev/tape/by-id/``. Names like ``/dev/sg0`` may point to a
   different device after reboot, and that is not what you want.

You can show the final configuration with::

 # proxmox-tape changer config sl3
 ┌──────┬─────────────────────────────┐
 │ Name │ Value                       │
 ╞══════╪═════════════════════════════╡
 │ name │ sl3                         │
 ├──────┼─────────────────────────────┤
 │ path │ /dev/tape/by-id/scsi-CC2C52 │
 └──────┴─────────────────────────────┘

Or simply list all configured changer devices::

 # proxmox-tape changer list
 ┌──────┬─────────────────────────────┬─────────┬──────────────┬────────────┐
 │ name │ path                        │ vendor  │ model        │ serial     │
 ╞══════╪═════════════════════════════╪═════════╪══════════════╪════════════╡
 │ sl3  │ /dev/tape/by-id/scsi-CC2C52 │ Quantum │ Superloader3 │ CC2C52     │
 └──────┴─────────────────────────────┴─────────┴──────────────┴────────────┘

The Vendor, Model and Serial number are auto detected, but only shown
if the device is online.

To test your setup, please query the status of the changer device with::

 # proxmox-tape changer status sl3
 ┌───────────────┬──────────┬────────────┬─────────────┐
 │ entry-kind    │ entry-id │ changer-id │ loaded-slot │
 ╞═══════════════╪══════════╪════════════╪═════════════╡
 │ drive         │        0 │ vtape1     │           1 │
 ├───────────────┼──────────┼────────────┼─────────────┤
 │ slot          │        1 │            │             │
 ├───────────────┼──────────┼────────────┼─────────────┤
 │ slot          │        2 │ vtape2     │             │
 ├───────────────┼──────────┼────────────┼─────────────┤
 │ ...           │      ... │            │             │
 ├───────────────┼──────────┼────────────┼─────────────┤
 │ slot          │       16 │            │             │
 └───────────────┴──────────┴────────────┴─────────────┘

Tape libraries usually provide some special import/export slots (also
called "mail slots"). Tapes inside those slots are acessible from
outside, making it easy to add/remove tapes to/from the library. Those
tapes are considered to be "offline", so backup jobs will not use
them. Those special slots are auto-detected and marked as
``import-export`` slot in the status command.

It's worth noting that some of the smaller tape libraries don't have
such slots. While they have something called "Mail Slot", that slot
is just a way to grab the tape from the gripper. But they are unable
to hold media while the robot does other things. They also do not
expose that "Mail Slot" over the SCSI interface, so you wont see them in
the status output.

As a workaround, you can mark some of the normal slots as export
slot. The software treats those slots like real ``import-export``
slots, and the media inside those slots is considered to be 'offline'
(not available for backup)::

 # proxmox-tape changer update sl3 --export-slots 15,16

After that, you can see those artificial ``import-export`` slots in
the status output::

 # proxmox-tape changer status sl3
 ┌───────────────┬──────────┬────────────┬─────────────┐
 │ entry-kind    │ entry-id │ changer-id │ loaded-slot │
 ╞═══════════════╪══════════╪════════════╪═════════════╡
 │ drive         │        0 │ vtape1     │           1 │
 ├───────────────┼──────────┼────────────┼─────────────┤
 │ import-export │       15 │            │             │
 ├───────────────┼──────────┼────────────┼─────────────┤
 │ import-export │       16 │            │             │
 ├───────────────┼──────────┼────────────┼─────────────┤
 │ slot          │        1 │            │             │
 ├───────────────┼──────────┼────────────┼─────────────┤
 │ slot          │        2 │ vtape2     │             │
 ├───────────────┼──────────┼────────────┼─────────────┤
 │ ...           │      ... │            │             │
 ├───────────────┼──────────┼────────────┼─────────────┤
 │ slot          │       14 │            │             │
 └───────────────┴──────────┴────────────┴─────────────┘


Tape drives
~~~~~~~~~~~

Linux is able to auto detect tape drives, and you can get a list
of available tape drives using::

 # proxmox-tape drive scan
 ┌────────────────────────────────┬────────┬─────────────┬────────┐
 │ path                           │ vendor │ model       │ serial │
 ╞════════════════════════════════╪════════╪═════════════╪════════╡
 │ /dev/tape/by-id/scsi-12345-nst │ IBM    │ ULT3580-TD4 │  12345 │
 └────────────────────────────────┴────────┴─────────────┴────────┘

In order to use that drive with Proxmox, you need to create a
configuration entry::

 # proxmox-tape drive create mydrive --path  /dev/tape/by-id/scsi-12345-nst

.. Note:: Please use stable device path names from inside
   ``/dev/tape/by-id/``. Names like ``/dev/nst0`` may point to a
   different device after reboot, and that is not what you want.

If you have a tape library, you also need to set the associated
changer device::

 # proxmox-tape drive update mydrive --changer sl3  --changer-drive-id 0

The ``--changer-drive-id`` is only necessary if the tape library
includes more than one drive (The changer status command lists all
drive IDs).

You can show the final configuration with::

 # proxmox-tape drive config mydrive
 ┌─────────┬────────────────────────────────┐
 │ Name    │ Value                          │
 ╞═════════╪════════════════════════════════╡
 │ name    │ mydrive                        │
 ├─────────┼────────────────────────────────┤
 │ path    │ /dev/tape/by-id/scsi-12345-nst │
 ├─────────┼────────────────────────────────┤
 │ changer │ sl3                            │
 └─────────┴────────────────────────────────┘

.. NOTE:: The ``changer-drive-id`` value 0 is not stored in the
   configuration, because that is the default.

To list all configured drives use::

 # proxmox-tape drive list
 ┌──────────┬────────────────────────────────┬─────────┬────────┬─────────────┬────────┐
 │ name     │ path                           │ changer │ vendor │ model       │ serial │
 ╞══════════╪════════════════════════════════╪═════════╪════════╪═════════════╪════════╡
 │ mydrive  │ /dev/tape/by-id/scsi-12345-nst │ sl3     │ IBM    │ ULT3580-TD4 │ 12345  │
 └──────────┴────────────────────────────────┴─────────┴────────┴─────────────┴────────┘

The Vendor, Model and Serial number are auto detected, but only shown
if the device is online.

For testing, you can simply query the drive status with::

 # proxmox-tape status --drive mydrive
 ┌───────────┬────────────────────────┐
 │ Name      │ Value                  │
 ╞═══════════╪════════════════════════╡
 │ blocksize │ 0                      │
 ├───────────┼────────────────────────┤
 │ status    │ DRIVE_OPEN | IM_REP_EN │
 └───────────┴────────────────────────┘

.. NOTE:: Blocksize should always be 0 (variable block size
   mode). This is the default anyways.


Media Pools
~~~~~~~~~~~

A media pool is a logical container for tapes. A backup job targets
one media pool, so a job only uses tapes from that pool.

.. topic:: Media Set

   A media set is a group of continuously written tapes, used to split
   the larger pool into smaller, restorable units. One or more backup
   jobs write to a media set, producing an ordered group of
   tapes. Media sets are identified by an unique ID. That ID and the
   sequence number is stored on each tape of that set (tape label).

   Media sets are the basic unit for restore tasks, i.e. you need all
   tapes in the set to restore the media set content. Data is fully
   deduplicated inside a media set.


.. topic:: Media Set Allocation Policy

   The pool additionally defines how long backup jobs can append data
   to a media set. The following settings are possible:

   - Try to use the current media set.

     This setting produce one large media set. While this is very
     space efficient (deduplication, no unused space), it can lead to
     long restore times, because restore jobs needs to read all tapes in the
     set.

     .. NOTE:: Data is fully deduplicated inside a media set. That
        also means that data is randomly distributed over the tapes in
        the set. So even if you restore a single VM, this may have to
        read data from all tapes inside the media set.

     Larger media sets are also more error prone, because a single
     damaged media makes the restore fail.

     Usage scenario: Mostly used with tape libraries, and you manually
     trigger new set creation by running a backup job with the
     ``--export`` option.

   - Always create a new media set.

     With this setting each backup job creates a new media set. This
     is less space efficient, because the last media from the last set
     may not be fully written, leaving the remaining space unused.

     The advantage is that this procudes media sets of minimal
     size. Small set are easier to handle, you can move sets to an
     off-site vault, and restore is much faster.

   - Create a new set when the specified Calendar Event triggers.

     .. _systemd.time manpage: https://manpages.debian.org/buster/systemd/systemd.time.7.en.html

     This allows you to specify points in time by using systemd like
     Calendar Event specifications (see `systemd.time manpage`_).

     For example, the value ``weekly`` (or ``Mon *-*-* 00:00:00``)
     will create a new set each week.

     This balances between space efficency and media count.

   Additionally, the following events may allocate a new media set:

   - Required tape is offline (and you use a tape library).

   - Current set contains damaged of retired tapes.

   - Database consistency errors, e.g. if the inventory does not
     contain required media info, or contain conflicting infos
     (outdated data).

.. topic:: Retention Policy

   Defines how long we want to keep the data.

   - Always overwrite media.

   - Protect data for the duration specified.

     We use systemd like time spans to specify durations, e.g. ``2
     weeks`` (see `systemd.time manpage`_).

   - Never overwrite data.

.. NOTE:: FIXME: Add note about global content namespace. (We do not store
   the source datastore, so it is impossible to distinguish
   store1:/vm/100 from store2:/vm/100. Please use different media
   pools if the source is from a different name space)


Empty Media Pool
^^^^^^^^^^^^^^^^

It is possible to label tapes with no pool assignment. Such tapes may
be used by any pool. Once used by a pool, media stays in that pool.



Tape Jobs
~~~~~~~~~


Administration
--------------

Label Tapes
~~~~~~~~~~~

By default, tape cartidges all looks the same, so you need to put a
label on them for unique identification. So first, put a sticky paper
label with some human readable text on the cartridge.

If you use a `Tape Library`_, you should use an 8 character string
encoded as `Code 39`_, as definded in the `LTO Ultrium Cartridge Label
Specification`_. You can either bye such barcode labels from your
cartidge vendor, or print them yourself. You can use our `LTO Barcode
Generator`_ App for that.

Next, you need to write that same label text to the tape, so that the
software can uniquely identify the tape too.

For a standalone drive, manually insert the new tape cartidge into the
drive and run:

 # proxmox-tape label --changer-id <label-text> --drive <drive-name>

.. Note:: For safety reasons, this command fails if the tape contain
   any data. If you want to overwrite it anways, erase the tape first.

You can verify success by reading back the label:

 # proxmox-tape read-label --drive <drive-name>

If you have a tape library, apply the sticky barcode label to the tape
cartridges first. Then load those empty tapes into the library. You
can then label all unlabeled tapes with a single command:

 # proxmox-tape barcode-label --drive <drive-name>


Run Tape Backups
~~~~~~~~~~~~~~~~

Restore from Tape
~~~~~~~~~~~~~~~~~

Update Inventory
~~~~~~~~~~~~~~~~

Restore Catalog
~~~~~~~~~~~~~~~
