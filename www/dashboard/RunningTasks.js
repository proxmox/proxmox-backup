Ext.define('PBS.RunningTasks', {
    extend: 'Ext.grid.Panel',
    alias: 'widget.pbsRunningTasks',

    title: gettext('Running Tasks'),
    emptyText: gettext('No running tasks'),

    hideHeaders: true,
    rowLines: false,

    scrollable: true,
    maxHeight: 500,

    viewConfig: {
	deferEmptyText: false,
    },

    controller: {
	xclass: 'Ext.app.ViewController',

	openTask: function(record) {
	    let me = this;
	    let view = me.getView();
	    Ext.create('Proxmox.window.TaskViewer', {
		upid: record.data.upid,
		endtime: record.data.endtime,
	    }).show();

	    view.fireEvent('taskopened', view, record.data.upid);
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
	sorters: 'starttime',
	rstore: PBS.data.RunningTasksStore,
    },

    columns: [
	{
	    text: gettext('Task'),
	    dataIndex: 'upid',
	    renderer: Proxmox.Utils.render_upid,
	    flex: 2,
	},
	{
	    text: gettext('Starttime'),
	    dataIndex: 'starttime',
	    renderer: function(value) {
		return Ext.Date.format(value, "Y-m-d H:i:s");
	    },
	    flex: 1,
	},
	{
	    text: gettext('Duration'),
	    dataIndex: 'duration',
	    renderer: function(value, md, record) {
		return Proxmox.Utils.format_duration_human((Date.now() - record.data.starttime)/1000);
	    },
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

