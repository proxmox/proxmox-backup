``pmtx help [{<command>}] [OPTIONS]``

Get help about specified command (or sub-command).

:``<command> <array>``:  Command. This may be a list in order to spefify nested sub-commands.


Optional parameters:

:``--verbose <boolean>``:  Verbose help.


----

``pmtx inquiry [OPTIONS]``

Inquiry

Optional parameters:

:``--changer <string>``:  Tape Changer Identifier.


:``--device <string>``:  Path to Linux generic SCSI device (e.g. '/dev/sg4')


:``--output-format <string>``:  Output format.


----

``pmtx inventory [OPTIONS]``

Inventory

Optional parameters:

:``--changer <string>``:  Tape Changer Identifier.


:``--device <string>``:  Path to Linux generic SCSI device (e.g. '/dev/sg4')


----

``pmtx load <slot> [OPTIONS]``

Load

:``<slot> <integer>``:  Storage slot number (source).


Optional parameters:

:``--changer <string>``:  Tape Changer Identifier.


:``--device <string>``:  Path to Linux generic SCSI device (e.g. '/dev/sg4')


:``--drivenum <integer>``:  Target drive number (defaults to Drive 0)


----

``pmtx status [OPTIONS]``

Changer Status

Optional parameters:

:``--changer <string>``:  Tape Changer Identifier.


:``--device <string>``:  Path to Linux generic SCSI device (e.g. '/dev/sg4')


:``--output-format <string>``:  Output format.


----

``pmtx transfer <from> <to> [OPTIONS]``

Transfer

:``<from> <integer>``:  Source storage slot number.


:``<to> <integer>``:  Target storage slot number.


Optional parameters:

:``--changer <string>``:  Tape Changer Identifier.


:``--device <string>``:  Path to Linux generic SCSI device (e.g. '/dev/sg4')


----

``pmtx unload [OPTIONS]``

Unload

Optional parameters:

:``--changer <string>``:  Tape Changer Identifier.


:``--device <string>``:  Path to Linux generic SCSI device (e.g. '/dev/sg4')


:``--drivenum <integer>``:  Target drive number (defaults to Drive 0)


:``--slot <integer>``:  Storage slot number (target). If omitted, defaults to the slot that the drive
  was loaded from.



