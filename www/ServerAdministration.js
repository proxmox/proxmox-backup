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
	    iconCls: 'fa fa-area-chart',
	},
	{
	    xtype: 'proxmoxNodeServiceView',
            title: gettext('Services'),
	    itemId: 'services',
	    iconCls: 'fa fa-cogs',
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
	    iconCls: 'fa fa-refresh',
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
	    iconCls: 'fa fa-list',
	    title: gettext('Syslog'),
	    url: "/api2/extjs/nodes/localhost/journal",
	},
	{
	    xtype: 'proxmoxNodeTasks',
	    itemId: 'tasks',
	    iconCls: 'fa fa-list-alt',
	    title: gettext('Tasks'),
	    height: 'auto',
	    nodename: 'localhost',
	},
    ],
});


