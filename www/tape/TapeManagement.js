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
	    title: gettext('Changers'),
	    itemId: 'changers',
	    xtype: 'pbsChangerStatus',
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
