Ext.define('PBS.SubscriptionKeyEdit', {
    extend: 'Proxmox.window.Edit',

    title: gettext('Upload Subscription Key'),
    width: 320,
    autoLoad: true,

    onlineHelp: 'get_help',

    items: {
	xtype: 'textfield',
	labelWidth: 120,
	name: 'key',
	value: '',
	fieldLabel: gettext('Subscription Key'),
    },
});

Ext.define('PBS.Subscription', {
    extend: 'Proxmox.grid.ObjectGrid',
    xtype: 'pbsSubscription',

    title: gettext('Subscription'),
    border: true,

    onlineHelp: 'get_help',
    tools: [PBS.Utils.get_help_tool("get_help")],

    viewConfig: {
	enableTextSelection: true,
    },

    showReport: function() {
	var me = this;

	var getReportFileName = function() {
	    var now = Ext.Date.format(new Date(), 'D-d-F-Y-G-i');
	    return `${me.nodename}-pbs-report-${now}.txt`;
	};

	var view = Ext.createWidget('component', {
	    itemId: 'system-report-view',
	    scrollable: true,
	    style: {
		'background-color': 'white',
		'white-space': 'pre',
		'font-family': 'monospace',
		padding: '5px',
	    },
	});

	var reportWindow = Ext.create('Ext.window.Window', {
	    title: gettext('System Report'),
	    width: 1024,
	    height: 600,
	    layout: 'fit',
	    modal: true,
	    buttons: [
		'->',
		{
		    text: gettext('Download'),
		    handler: function() {
			var fileContent = Ext.String.htmlDecode(reportWindow.getComponent('system-report-view').html);
			var fileName = getReportFileName();

			// Internet Explorer
			if (window.navigator.msSaveOrOpenBlob) {
			    navigator.msSaveOrOpenBlob(new Blob([fileContent]), fileName);
			} else {
			    var element = document.createElement('a');
			    element.setAttribute('href', 'data:text/plain;charset=utf-8,' +
			      encodeURIComponent(fileContent));
			    element.setAttribute('download', fileName);
			    element.style.display = 'none';
			    document.body.appendChild(element);
			    element.click();
			    document.body.removeChild(element);
			}
		    },
		},
	    ],
	    items: view,
	});

	Proxmox.Utils.API2Request({
	    url: '/api2/extjs/nodes/' + me.nodename + '/report',
	    method: 'GET',
	    waitMsgTarget: me,
	    failure: function(response) {
		Ext.Msg.alert(gettext('Error'), response.htmlStatus);
	    },
	    success: function(response) {
		var report = Ext.htmlEncode(response.result.data);
		reportWindow.show();
		view.update(report);
	    },
	});
    },

    initComponent: function() {
	let me = this;

	let reload = () => me.rstore.load();
	let baseurl = '/nodes/localhost/subscription';

	let rows = {
	    productname: {
		header: gettext('Type'),
	    },
	    key: {
		header: gettext('Subscription Key'),
	    },
	    status: {
		header: gettext('Status'),
		renderer: (value) => {
		    value = Ext.String.capitalize(value);
		    let message = me.getObjectValue('message');
		    if (message) {
			return value + ": " + message;
		    }
		    return value;
		},
	    },
	    message: {
		visible: false,
	    },
	    serverid: {
		header: gettext('Server ID'),
	    },
	    checktime: {
		header: gettext('Last checked'),
		renderer: Proxmox.Utils.render_timestamp,
	    },
	    nextduedate: {
		header: gettext('Next due date'),
	    },
	    signature: {
		header: gettext('Signed/Offline'),
		renderer: (value) => {
		    if (value) {
			return gettext('Yes');
		    } else {
			return gettext('No');
		    }
		},
	    },
	};

	Ext.apply(me, {
	    url: `/api2/json${baseurl}`,
	    cwidth1: 170,
	    tbar: [
		{
		    text: gettext('Upload Subscription Key'),
		    iconCls: 'fa fa-ticket',
		    handler: function() {
			let win = Ext.create('PBS.SubscriptionKeyEdit', {
			    url: '/api2/extjs/' + baseurl,
			    autoShow: true,
			});
			win.on('destroy', reload);
		    },
		},
		{
		    text: gettext('Check'),
		    iconCls: 'fa fa-check-square-o',
		    handler: function() {
			Proxmox.Utils.API2Request({
			    params: { force: 1 },
			    url: baseurl,
			    method: 'POST',
			    waitMsgTarget: me,
			    failure: function(response, opts) {
				Ext.Msg.alert(gettext('Error'), response.htmlStatus);
			    },
			    callback: reload,
			});
		    },
		},
		{
		    text: gettext('Remove Subscription'),
		    xtype: 'proxmoxStdRemoveButton',
		    confirmMsg: gettext('Are you sure you want to remove the subscription key?'),
		    baseurl: baseurl,
		    dangerous: true,
		    selModel: false,
		    callback: reload,
		    iconCls: 'fa fa-trash-o',
		},
		'-',
		{
		    text: gettext('System Report'),
		    iconCls: 'fa fa-stethoscope',
		    handler: function() {
			Proxmox.Utils.checked_command(function() { me.showReport(); });
		    },
		},
	    ],
	    rows: rows,
	});

	me.callParent();

	reload();
    },
});
