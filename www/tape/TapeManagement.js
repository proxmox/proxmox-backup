Ext.define('PBS.TapeManagement', {
    extend: 'Ext.tab.Panel',
    alias: 'widget.pbsTapeManagement',

    title: gettext('Tape Backup'),

    tools: [PBS.Utils.get_help_tool("tape_backup")],

    border: true,
    defaults: {
	border: false,
	xtype: 'panel',
    },

    items: [
	{
	    xtype: 'pbsBackupOverview',
	    title: gettext('Content'),
	    itemId: 'content',
	},
	{
	    xtype: 'pbsTapeInventory',
	    title: gettext('Inventory'),
	    itemId: 'inventory',
	},
	{
	    xtype: 'pbsTapeChangerPanel',
	    title: gettext('Changers'),
	    itemId: 'changers',
	},
	{
	    xtype: 'pbsTapeDrivePanel',
	    title: gettext('Drives'),
	    itemId: 'drives',
	},
	{
	    title: gettext('Media Pools'),
	    itemId: 'pools',
	    xtype: 'pbsMediaPoolPanel',
	},
	{
	    xtype: 'pbsEncryptionKeys',
	    title: gettext('Encryption Keys'),
	    itemId: 'encryption-keys',
	},
	{
	    xtype: 'pbsTapeBackupJobView',
	    title: gettext('Backup Jobs'),
	    itemId: 'tape-backup-jobs',
	},
    ],
});
