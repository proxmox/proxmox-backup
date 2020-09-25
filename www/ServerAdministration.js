Ext.define('PBS.ServerAdministration', {
    extend: 'Ext.tab.Panel',
    alias: 'widget.pbsServerAdministration',

    title: gettext('Server Administration'),

    border: true,
    defaults: { border: false },

    controller: {
	xclass: 'Ext.app.ViewController',

        init: function(view) {
	    var upgradeBtn = view.lookupReference('upgradeBtn');
	    upgradeBtn.setDisabled(!(Proxmox.UserName && Proxmox.UserName === 'root@pam'));
	},
    },

    items: [
	{
	    xtype: 'pbsServerStatus',
	    itemId: 'status',
	},
	{
	    xtype: 'proxmoxNodeServiceView',
            title: gettext('Services'),
	    itemId: 'services',
	    restartCommand: 'reload', // avoid disruptions
	    startOnlyServices: {
		syslog: true,
		'proxmox-backup': true,
		'proxmox-backup-proxy': true,
	    },
	    nodename: 'localhost',
	},
	{
	    xtype: 'proxmoxNodeAPT',
            title: gettext('Updates'),
	    upgradeBtn: {
		xtype: 'button',
		reference: 'upgradeBtn',
		disabled: true,
		text: gettext('Upgrade'),
		handler: function() {
		    Proxmox.Utils.openXtermJsViewer('upgrade', 0, 'localhost');
		},
	    },
	    itemId: 'updates',
	    nodename: 'localhost',
	},
	{
	    xtype: 'proxmoxJournalView',
	    itemId: 'logs',
	    title: gettext('Syslog'),
	    url: "/api2/extjs/nodes/localhost/journal",
	},
	{
	    xtype: 'proxmoxNodeTasks',
	    itemId: 'tasks',
	    title: gettext('Tasks'),
	    height: 'auto',
	    nodename: 'localhost',
	},
    ],
});


