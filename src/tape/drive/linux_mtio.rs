//! Linux Magnetic Tape Driver ioctl definitions
//!
//! from: /usr/include/x86_64-linux-gnu/sys/mtio.h
//!
//! also see: man 4 st

#[repr(C)]
pub struct mtop {
    pub mt_op: MTCmd,		/* Operations defined below.  */
    pub mt_count: libc::c_int,		/* How many of them.  */
}

#[repr(i16)]
#[allow(dead_code)] // do not warn about unused command
pub enum MTCmd {
    MTRESET = 0,	/* +reset drive in case of problems */
    MTFSF = 1,  	/* forward space over FileMark,
			 * position at first record of next file
			 */
    MTBSF = 2,  	/* backward space FileMark (position before FM) */
    MTFSR = 3,	        /* forward space record */
    MTBSR = 4,	        /* backward space record */
    MTWEOF = 5,	        /* write an end-of-file record (mark) */
    MTREW = 6,	        /* rewind */
    MTOFFL = 7,	        /* rewind and put the drive offline (eject?) */
    MTNOP = 8,	        /* no op, set status only (read with MTIOCGET) */
    MTRETEN = 9,	/* retension tape */
    MTBSFM = 10,	/* +backward space FileMark, position at FM */
    MTFSFM = 11,	/* +forward space FileMark, position at FM */
    MTEOM = 12,         /* goto end of recorded media (for appending files).
			 * MTEOM positions after the last FM, ready for
			 * appending another file.
			 */
    MTERASE = 13,	/* erase tape -- be careful! */
    MTRAS1 = 14, 	/* run self test 1 (nondestructive) */
    MTRAS2 = 15,	/* run self test 2 (destructive) */
    MTRAS3 = 16,	/* reserved for self test 3 */
    MTSETBLK = 20,	/* set block length (SCSI) */
    MTSETDENSITY = 21,	/* set tape density (SCSI) */
    MTSEEK = 22, 	/* seek to block (Tandberg, etc.) */
    MTTELL = 23,        /* tell block (Tandberg, etc.) */
    MTSETDRVBUFFER = 24,/* set the drive buffering according to SCSI-2 */

    /* ordinary buffered operation with code 1 */
    MTFSS = 25,	        /* space forward over setmarks */
    MTBSS = 26,	        /* space backward over setmarks */
    MTWSM = 27,	        /* write setmarks */

    MTLOCK = 28,	/* lock the drive door */
    MTUNLOCK = 29,	/* unlock the drive door */
    MTLOAD = 30,	/* execute the SCSI load command */
    MTUNLOAD = 31,	/* execute the SCSI unload command */
    MTCOMPRESSION = 32, /* control compression with SCSI mode page 15 */
    MTSETPART = 33,	/* Change the active tape partition */
    MTMKPART = 34,	/* Format the tape with one or two partitions */
    MTWEOFI = 35,	/* write an end-of-file record (mark) in immediate mode */
}

//#define	MTIOCTOP	_IOW('m', 1, struct mtop)	/* Do a mag tape op. */
nix::ioctl_write_ptr!(mtioctop, b'm', 1, mtop);

// from: /usr/include/x86_64-linux-gnu/sys/mtio.h
#[derive(Default, Debug)]
#[repr(C)]
pub struct mtget {
    pub mt_type: libc::c_long,		/* Type of magtape device.  */
    pub mt_resid: libc::c_long,		/* Residual count: (not sure)
				   number of bytes ignored, or
				   number of files not skipped, or
				   number of records not skipped.  */
    /* The following registers are device dependent.  */
    pub mt_dsreg: libc::c_long,		/* Status register.  */
    pub mt_gstat: libc::c_long,		/* Generic (device independent) status.  */
    pub mt_erreg: libc::c_long,		/* Error register.  */
    /* The next two fields are not always used.  */
    pub mt_fileno: i32     ,	/* Number of current file on tape.  */
    pub mt_blkno: i32,		/* Current block number.  */
}

//#define	MTIOCGET	_IOR('m', 2, struct mtget)	/* Get tape status.  */
nix::ioctl_read!(mtiocget, b'm', 2, mtget);

#[repr(C)]
#[allow(dead_code)]
pub struct mtpos {
    pub mt_blkno: libc::c_long,	 /* current block number */
}

//#define	MTIOCPOS	_IOR('m', 3, struct mtpos)	/* Get tape position.*/
nix::ioctl_read!(mtiocpos, b'm', 3, mtpos);

pub const MT_ST_BLKSIZE_MASK: libc::c_long = 0x0ffffff;
pub const MT_ST_BLKSIZE_SHIFT: usize = 0;
pub const MT_ST_DENSITY_MASK: libc::c_long = 0xff000000;
pub const MT_ST_DENSITY_SHIFT: usize = 24;

pub const MT_TYPE_ISSCSI1: libc::c_long = 0x71;	/* Generic ANSI SCSI-1 tape unit.  */
pub const MT_TYPE_ISSCSI2: libc::c_long = 0x72;	/* Generic ANSI SCSI-2 tape unit.  */

// Generic Mag Tape (device independent) status macros for examining mt_gstat -- HP-UX compatible
// from: /usr/include/x86_64-linux-gnu/sys/mtio.h
bitflags::bitflags!{
   pub struct GMTStatusFlags: libc::c_long {
       const EOF = 0x80000000;
       const BOT = 0x40000000;
       const EOT = 0x20000000;
       const SM  = 0x10000000;  /* DDS setmark */
       const EOD = 0x08000000;  /* DDS EOD */
       const WR_PROT = 0x04000000;

       const ONLINE = 0x01000000;
       const D_6250 = 0x00800000;
       const D_1600 = 0x00400000;
       const D_800 = 0x00200000;
       const DRIVE_OPEN = 0x00040000;  /* Door open (no tape).  */
       const IM_REP_EN =  0x00010000;  /* Immediate report mode.*/
       const END_OF_STREAM = 0b00000001;
   }
}

#[repr(i32)]
#[allow(non_camel_case_types, dead_code)]
pub enum SetDrvBufferCmd {
    MT_ST_BOOLEANS =         0x10000000,
    MT_ST_SETBOOLEANS =	     0x30000000,
    MT_ST_CLEARBOOLEANS	=    0x40000000,
    MT_ST_WRITE_THRESHOLD =  0x20000000,
    MT_ST_DEF_BLKSIZE =      0x50000000,
    MT_ST_DEF_OPTIONS =	     0x60000000,
    MT_ST_SET_TIMEOUT =	     0x70000000,
    MT_ST_SET_LONG_TIMEOUT = 0x70100000,
    MT_ST_SET_CLN =          0x80000000u32 as i32,
}

bitflags::bitflags!{
   pub struct SetDrvBufferOptions: i32 {
       const MT_ST_BUFFER_WRITES =    0x1;
       const MT_ST_ASYNC_WRITES =     0x2;
       const MT_ST_READ_AHEAD	=     0x4;
       const MT_ST_DEBUGGING =        0x8;
       const MT_ST_TWO_FM =          0x10;
       const MT_ST_FAST_MTEOM	=    0x20;
       const MT_ST_AUTO_LOCK =       0x40;
       const MT_ST_DEF_WRITES =      0x80;
       const MT_ST_CAN_BSR =        0x100;
       const MT_ST_NO_BLKLIMS =     0x200;
       const MT_ST_CAN_PARTITIONS = 0x400;
       const MT_ST_SCSI2LOGICAL =   0x800;
       const MT_ST_SYSV =          0x1000;
       const MT_ST_NOWAIT =        0x2000;
       const MT_ST_SILI =  	   0x4000;
   }
}
