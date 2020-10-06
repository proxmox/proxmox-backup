Ext.define('PBS.LongestTasks', {
    extend: 'Ext.grid.Panel',
    alias: 'widget.pbsLongestTasks',

    title: gettext('Longest Tasks'),

    hideHeaders: true,
    rowLines: false,

    emptyText: gettext('No Tasks'),

    controller: {
	xclass: 'Ext.app.ViewController',

	openTask: function(record) {
	    let me = this;
	    Ext.create('Proxmox.window.TaskViewer', {
		upid: record.data.upid,
		endtime: record.data.endtime,
	    }).show();
	},

	openTaskItemDblClick: function(grid, record) {
	    this.openTask(record);
	},

	openTaskActionColumn: function(grid, rowIndex) {
	    this.openTask(grid.getStore().getAt(rowIndex));
	},

	render_status: function(value) {
	    let cls = 'times-circle critical';
	    if (value === 'OK') {
		cls = 'check-circle good';
	    } else if (value.startsWith('WARNINGS:')) {
		cls = 'exclamation-circle warning';
	    } else if (value === 'unknown') {
		cls = 'question-circle faded';
	    }

	    return `<i class="fa fa-${cls}"></i>`;
	},
    },

    updateTasks: function(data) {
	let me = this;
	me.getStore().setData(data);
    },

    listeners: {
	itemdblclick: 'openTaskItemDblClick',
    },

    store: {
	type: 'diff',
	autoDestroy: true,
	autoDestroyRstore: true,
	sorters: [
	    {
		property: 'duration',
		direction: 'DESC',
	    },
	    {
		property: 'upid',
		direction: 'ASC',
	    },
	],
	rstore: {
	    storeid: 'proxmox-tasks-dash',
	    type: 'store',
	    model: 'proxmox-tasks',
	    proxy: {
		type: 'memory',
	    },
	},
    },

    columns: [
	{
	    text: gettext('Task'),
	    dataIndex: 'upid',
	    renderer: Proxmox.Utils.render_upid,
	    flex: 1,
	},
	{
	    text: gettext('Duration'),
	    dataIndex: 'duration',
	    renderer: Proxmox.Utils.format_duration_human,
	},
	{
	    text: gettext('Status'),
	    align: 'center',
	    width: 40,
	    dataIndex: 'status',
	    renderer: 'render_status',
	},
	{
	    xtype: 'actioncolumn',
	    width: 40,
	    items: [
		{
		    iconCls: 'fa fa-chevron-right',
		    tooltip: gettext('Open Task'),
		    handler: 'openTaskActionColumn',
		},
	    ],
	},
    ],
});
