Tape Backup
===========

Our tape backup solution provides a easy way to store datastore
contents on a tape. This increases data safety because you get:

- an additional copy of the data
- to a different media type (tape)
- to an additional location (you can move tape offsite)

Tape backups do not provide random access to the stored
data. Instead, you need to restore the data to disk before you can
access it again.


Tape Technology Primer
----------------------

.. _Linear Tape Open: https://en.wikipedia.org/wiki/Linear_Tape-Open

As of 2021, the only broadly available tape technology standard is
`Linear Tape Open`_, and different vendors offers LTO Ultrium tape
drives and autoloaders.

In general, LTO tapes offer the following advantages:

- Durable (30 years)
- High Capacity (12 TB)
- Relatively low cost per TB
- Cold Media
- Movable (storable inside vault)
- Multiple vendors (for both media and drives)


Supported Hardware
------------------

Proxmox Backup Server supports `Linear Tape Open`_ genertion 3
(LTO3) or later. In general, all SCSI2 tape drives supported by
the Linux kernel should work.

Tape changer support is done using the Linux 'mtx' command line
tool. So any changer devive supported by that tool work work.



Terminology
-----------

:Tape Labels: are used to uniquely indentify a tape. You normally use
   some sticky paper labels and apply them on the front of the
   cartridge. We additionally store the label text magnetically on the
   tape (first file on tape).

.. _Code 39: https://en.wikipedia.org/wiki/Code_39

.. _LTO Ultrium Cartridge Label Specification: https://www.ibm.com/support/pages/ibm-lto-ultrium-cartridge-label-specification

:Barcodes: are a special form of tape labels, which are electronically
   readable. Most LTO tape robots use an 8 character string encoded as
   `Code 39`_, as definded in the `LTO Ultrium Cartridge Label
   Specification`_.

   You can either bye such barcode labels from your cartidge vendor,
   or print them yourself.

.. Note:: Physical labels and the associated adhesive shall have an
   environmental performance to match or exceed the environmental
   specifications of the cartridge to which it is applied.

:Media Pools: A media pool is a logical container for tapes. A backup
   job targets one media pool, so a job only uses tapes from that
   pool. The pool aditionally defines how long we can append data to a
   tape (allocation policy), and how long we want to keep that data
   (retention policy).

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
