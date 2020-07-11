Ext.define('PBS.data.RunningTasksStore', {
    extend: 'Proxmox.data.UpdateStore',

    singleton: true,

    constructor: function(config) {
	let me = this;
	config = config || {};
	Ext.apply(config, {
	    interval: 3000,
	    storeid: 'pbs-running-tasks-dash',
	    model: 'proxmox-tasks',
	    proxy: {
		type: 'proxmox',
		// maybe separate api call?
		url: '/api2/json/nodes/localhost/tasks?running=1&limit=100',
	    },
	});
	me.callParent([config]);
    },
});
