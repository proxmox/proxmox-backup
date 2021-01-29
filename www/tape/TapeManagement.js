Ext.define('PBS.TapeManagement', {
    extend: 'Ext.tab.Panel',
    alias: 'widget.pbsTapeManagement',

    title: gettext('Tape Backup'),

    border: true,
    defaults: {
	border: false,
	xtype: 'panel',
    },

    items: [
	{
	    title: gettext('Backup'),
	    itemId: 'backup',
	    xtype: 'pbsBackupOverview',
	},
	{
	    title: gettext('Library'),
	    itemId: 'library',
	    xtype: 'pbsChangerStatus',
	},
	{
	    title: gettext('Changers'),
	    itemId: 'changers',
	    xtype: 'pbsTapeChangerPanel',
	},
	{
	    title: gettext('Drives'),
	    itemId: 'drives',
	    xtype: 'pbsTapeDrivePanel',
	},
	{
	    title: gettext('Media Pools'),
	    itemId: 'pools',
	    xtype: 'pbsMediaPoolPanel',
	},
    ],
});
