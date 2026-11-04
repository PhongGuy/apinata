// PAC helper functions, rewritten after the dist/ helpers of pac-resolver 7.0.1 (MIT) and, for
// isInNet's masks, netmask 2.0.2 (MIT).
// Attribution and licence: THIRD-PARTY.md.
(function (global) {
  // The host installs its DNS and address functions under private names; capture them and
  // remove the names so a script sees only the standard helpers.
  var hostResolve = global.__pacDnsResolve;
  var hostIsInNet = global.__pacIsInNet;
  var hostMyIp = global.__pacMyIpAddress;
  delete global.__pacDnsResolve;
  delete global.__pacIsInNet;
  delete global.__pacMyIpAddress;

  var weekdays = ['SUN', 'MON', 'TUE', 'WED', 'THU', 'FRI', 'SAT'];

  function inRange(start, value, finish) {
    return start <= value && value <= finish;
  }

  global.isPlainHostName = function isPlainHostName(host) {
    return !/\./.test(host);
  };

  global.dnsDomainIs = function dnsDomainIs(host, domain) {
    host = String(host);
    domain = String(domain);
    return host.substr(domain.length * -1) === domain;
  };

  global.localHostOrDomainIs = function localHostOrDomainIs(host, hostdom) {
    var parts = host.split('.');
    var domparts = hostdom.split('.');
    for (var i = 0; i < parts.length; i++) {
      if (parts[i] !== domparts[i]) return false;
    }
    return true;
  };

  global.dnsDomainLevels = function dnsDomainLevels(host) {
    var match = String(host).match(/\./g);
    return match ? match.length : 0;
  };

  global.shExpMatch = function shExpMatch(str, shexp) {
    var source = String(shexp)
      .replace(/\./g, '\\.')
      .replace(/\?/g, '.')
      .replace(/\*/g, '.*');
    return new RegExp('^' + source + '$').test(str);
  };

  // The host answers undefined when a name has no IPv4 address.
  global.dnsResolve = function dnsResolve(host) {
    var ip = hostResolve(String(host));
    return ip === undefined ? null : ip;
  };

  global.isResolvable = function isResolvable(host) {
    return hostResolve(String(host)) !== undefined;
  };

  // As netmask 2.0.2: a missing mask may follow a slash in the pattern and defaults to 32, and a
  // mask without a dot is a bit count read by parseInt. The host gets a dotted mask or a count.
  global.isInNet = function isInNet(host, pattern, mask) {
    if (typeof pattern !== 'string') return false;
    if (!mask) {
      var parts = pattern.split('/', 2);
      pattern = parts[0];
      mask = parts[1] || 32;
    }
    if (!(typeof mask === 'string' && mask.indexOf('.') > -1)) {
      var bits = parseInt(mask, 10);
      if (!(bits <= 32)) return false;
      mask = String(Math.max(bits, 0));
    }
    return hostIsInNet(String(host), pattern, mask);
  };

  global.myIpAddress = function myIpAddress() {
    return hostMyIp();
  };

  global.weekdayRange = function weekdayRange(wd1, wd2, gmt) {
    var useGmt = false;
    var wd2IsGmt = false;
    if (gmt === 'GMT') {
      useGmt = true;
    } else if (wd2 === 'GMT') {
      useGmt = true;
      wd2IsGmt = true;
    }
    var wd1Index = weekdays.indexOf(wd1);
    var wd2Index = -1;
    if (!wd2IsGmt && wd2 && weekdays.includes(wd2)) {
      wd2Index = weekdays.indexOf(wd2);
    }
    var today = useGmt ? new Date().getUTCDay() : new Date().getDay();
    if (wd2Index < 0) return today === wd1Index;
    if (wd1Index <= wd2Index) return inRange(wd1Index, today, wd2Index);
    return inRange(wd1Index, today, 6) || inRange(0, today, wd2Index);
  };

  global.timeRange = function timeRange() {
    var args = Array.prototype.slice.call(arguments);
    var last = args.pop();
    var useGmt = last === 'GMT';
    var now = new Date();
    if (!useGmt) args.push(last);
    var n = args.map(function (v) { return parseInt(v, 10); });
    var hour = useGmt ? now.getUTCHours() : now.getHours();
    var minute = useGmt ? now.getUTCMinutes() : now.getMinutes();
    var second = useGmt ? now.getUTCSeconds() : now.getSeconds();
    function seconds(hh, mm, ss) {
      return hh * 3600 + mm * 60 + ss;
    }
    switch (args.length) {
      case 1:
        return hour === n[0];
      case 2:
        return n[0] <= hour && hour < n[1];
      case 4:
        return inRange(seconds(n[0], n[1], 0), seconds(hour, minute, 0), seconds(n[2], n[3], 59));
      case 6:
        return inRange(
          seconds(n[0], n[1], n[2]),
          seconds(hour, minute, second),
          seconds(n[3], n[4], n[5])
        );
      default:
        return false;
    }
  };

  global.dateRange = function dateRange() {
    return false;
  };

  global.alert = function alert() {};
})(globalThis);
